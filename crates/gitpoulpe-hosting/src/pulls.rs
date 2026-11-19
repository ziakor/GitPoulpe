use gitpoulpe_core::Connection;
use serde_json::{Value, json};

use crate::github::{GithubError, interpret_failure};
use crate::http::ApiClient;
use crate::repos::{RepoSummary, client, parse_repo, valid_org};
use crate::secrets::SecretStore;

const MAX_FILES: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comparison {
    pub ahead: u64,
    pub behind: u64,
    pub files: Vec<ChangedFile>,
    pub files_truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostedPull {
    pub number: u64,
    pub url: String,
    pub title: String,
    pub draft: bool,
    pub state: String,
    pub head: String,
    pub base: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullTarget {
    pub head_repo: RepoSummary,
    pub target_repo: RepoSummary,
    pub branch: String,
    pub base: String,
    pub published: bool,
    pub comparison: Option<Comparison>,
    pub existing: Option<HostedPull>,
}

impl PullTarget {
    pub fn cross_repo(&self) -> bool {
        self.head_repo.full_name().to_lowercase() != self.target_repo.full_name().to_lowercase()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullDraft {
    pub title: String,
    pub body: String,
    pub draft: bool,
}

fn valid_branch(branch: &str) -> bool {
    !branch.is_empty()
        && branch.len() <= 250
        && !branch.starts_with('/')
        && !branch.contains("//")
        && !branch.contains("..")
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn get_repo(
    client: &ApiClient,
    connection: &Connection,
    owner: &str,
    name: &str,
) -> Result<RepoSummary, GithubError> {
    let response = client.get(&format!("repos/{owner}/{name}")).await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    let value = response.json().map_err(|_| GithubError::Unreadable)?;
    parse_repo(connection, &value).ok_or(GithubError::Unreadable)
}

fn parse_pull(value: &Value) -> Option<HostedPull> {
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    Some(HostedPull {
        number: value.get("number")?.as_u64()?,
        url: text(value, "html_url")?,
        title: text(value, "title").unwrap_or_default(),
        draft: value.get("draft").and_then(Value::as_bool).unwrap_or(false),
        state: text(value, "state").unwrap_or_default(),
        head: value
            .get("head")
            .and_then(|h| text(h, "ref"))
            .unwrap_or_default(),
        base: value
            .get("base")
            .and_then(|b| text(b, "ref"))
            .unwrap_or_default(),
    })
}

async fn branch_published(
    client: &ApiClient,
    owner: &str,
    name: &str,
    branch: &str,
) -> Result<bool, GithubError> {
    let response = client
        .get(&format!("repos/{owner}/{name}/git/ref/heads/{branch}"))
        .await?;
    match response.status {
        404 => Ok(false),
        _ if response.ok() => Ok(true),
        _ => Err(interpret_failure(&response)),
    }
}

async fn open_pull_for(
    client: &ApiClient,
    target: &RepoSummary,
    head_owner: &str,
    branch: &str,
    base: &str,
) -> Result<Option<HostedPull>, GithubError> {
    let head = encode(&format!("{head_owner}:{branch}"));
    let response = client
        .get(&format!(
            "repos/{}/{}/pulls?state=open&head={head}&base={}",
            target.owner,
            target.name,
            encode(base)
        ))
        .await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    let value = response.json().map_err(|_| GithubError::Unreadable)?;
    Ok(value
        .as_array()
        .and_then(|list| list.iter().find_map(parse_pull)))
}

fn head_ref(head: &RepoSummary, target: &RepoSummary, branch: &str) -> String {
    if head.full_name().eq_ignore_ascii_case(&target.full_name()) {
        branch.to_owned()
    } else {
        format!("{}:{branch}", head.owner)
    }
}

pub async fn prepare_pull(
    store: &dyn SecretStore,
    connection: &Connection,
    head_owner: &str,
    head_name: &str,
    branch: &str,
    to_parent: bool,
    base: Option<&str>,
) -> Result<PullTarget, GithubError> {
    if !valid_org(head_owner)
        || !valid_org(head_name)
        || !valid_branch(branch)
        || base.is_some_and(|b| !valid_branch(b))
    {
        return Err(GithubError::NotFound);
    }
    let client = client(store, connection)?;
    let head_repo = get_repo(&client, connection, head_owner, head_name).await?;
    let target_repo = match head_repo.parent.as_deref().filter(|_| to_parent) {
        Some(parent) => {
            let (owner, name) = parent.split_once('/').ok_or(GithubError::Unreadable)?;
            get_repo(&client, connection, owner, name).await?
        }
        None => head_repo.clone(),
    };
    let base = match base {
        Some(base) => base.to_owned(),
        None => target_repo
            .default_branch
            .clone()
            .ok_or(GithubError::Unreadable)?,
    };
    let published = branch_published(&client, &head_repo.owner, &head_repo.name, branch).await?;
    let (comparison, existing) = if published {
        let head = head_ref(&head_repo, &target_repo, branch);
        let response = client
            .get(&format!(
                "repos/{}/{}/compare/{}...{}",
                target_repo.owner,
                target_repo.name,
                encode(&base),
                encode(&head)
            ))
            .await?;
        if !response.ok() {
            return Err(interpret_failure(&response));
        }
        let value = response.json().map_err(|_| GithubError::Unreadable)?;
        let files: Vec<ChangedFile> = value
            .get("files")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|f| {
                        Some(ChangedFile {
                            path: f.get("filename")?.as_str()?.to_owned(),
                            status: f.get("status")?.as_str()?.to_owned(),
                            additions: f.get("additions").and_then(Value::as_u64).unwrap_or(0),
                            deletions: f.get("deletions").and_then(Value::as_u64).unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let comparison = Comparison {
            ahead: value.get("ahead_by").and_then(Value::as_u64).unwrap_or(0),
            behind: value.get("behind_by").and_then(Value::as_u64).unwrap_or(0),
            files_truncated: files.len() >= MAX_FILES,
            files,
        };
        let existing =
            open_pull_for(&client, &target_repo, &head_repo.owner, branch, &base).await?;
        (Some(comparison), existing)
    } else {
        (None, None)
    };
    Ok(PullTarget {
        head_repo,
        target_repo,
        branch: branch.to_owned(),
        base,
        published,
        comparison,
        existing,
    })
}

pub async fn create_pull_request(
    store: &dyn SecretStore,
    connection: &Connection,
    prepared: &PullTarget,
    draft: &PullDraft,
) -> Result<HostedPull, GithubError> {
    let title = draft.title.trim();
    if title.is_empty() {
        return Err(GithubError::Validation(
            "le titre est obligatoire".to_owned(),
        ));
    }
    if !prepared.published {
        return Err(GithubError::BranchNotPublished(prepared.branch.clone()));
    }
    let client = client(store, connection)?;
    if let Some(existing) = open_pull_for(
        &client,
        &prepared.target_repo,
        &prepared.head_repo.owner,
        &prepared.branch,
        &prepared.base,
    )
    .await?
    {
        return Err(GithubError::PullExists(existing.number, existing.url));
    }
    let head = head_ref(&prepared.head_repo, &prepared.target_repo, &prepared.branch);
    let response = client
        .send(
            "POST",
            &format!(
                "repos/{}/{}/pulls",
                prepared.target_repo.owner, prepared.target_repo.name
            ),
            Some(json!({
                "title": title,
                "body": draft.body,
                "head": head,
                "base": prepared.base,
                "draft": draft.draft,
            })),
        )
        .await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    let value = response.json().map_err(|_| GithubError::Unreadable)?;
    parse_pull(&value).ok_or(GithubError::Unreadable)
}
