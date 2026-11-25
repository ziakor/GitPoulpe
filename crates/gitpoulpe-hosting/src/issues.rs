use gitpoulpe_core::{Connection, Provider};
use serde_json::Value;

use crate::github::{GithubError, interpret_failure};
use crate::http::{ApiClient, next_link};
use crate::pulls::encode;
use crate::repos::{client, pages, valid_org};
use crate::secrets::SecretStore;

const MAX_SEARCH_PAGES: usize = 10;
const MAX_NOTES: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueFilter {
    Open,
    Closed,
    All,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueSummary {
    pub number: u64,
    pub title: String,
    pub open: bool,
    pub author: String,
    pub labels: Vec<String>,
    pub comments: u64,
    pub updated_at: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueNote {
    pub author: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssueLink {
    Pull {
        number: u64,
        title: String,
        open: bool,
    },
    Commit {
        sha: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueDetail {
    pub summary: IssueSummary,
    pub body: String,
    pub notes: Vec<IssueNote>,
    pub notes_truncated: bool,
    pub links: Vec<IssueLink>,
    pub instance: Option<String>,
    pub limits: Vec<String>,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn user(value: &Value, key: &str, field: &str) -> String {
    value
        .get(key)
        .and_then(|u| text(u, field))
        .unwrap_or_default()
}

fn labels(value: &Value) -> Vec<String> {
    value
        .get("labels")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|l| match l {
                    Value::String(name) => Some(name.clone()),
                    other => text(other, "name"),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn github_summary(value: &Value) -> Option<IssueSummary> {
    if value.get("pull_request").is_some() {
        return None;
    }
    Some(IssueSummary {
        number: value.get("number")?.as_u64()?,
        title: text(value, "title").unwrap_or_default(),
        open: text(value, "state").as_deref() == Some("open"),
        author: user(value, "user", "login"),
        labels: labels(value),
        comments: value.get("comments").and_then(Value::as_u64).unwrap_or(0),
        updated_at: text(value, "updated_at").unwrap_or_default(),
        url: text(value, "html_url").unwrap_or_default(),
    })
}

fn gitlab_summary(value: &Value) -> Option<IssueSummary> {
    Some(IssueSummary {
        number: value.get("iid")?.as_u64()?,
        title: text(value, "title").unwrap_or_default(),
        open: text(value, "state").as_deref() == Some("opened"),
        author: user(value, "author", "username"),
        labels: labels(value),
        comments: value
            .get("user_notes_count")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        updated_at: text(value, "updated_at").unwrap_or_default(),
        url: text(value, "web_url").unwrap_or_default(),
    })
}

fn failure(response: &crate::http::Response) -> GithubError {
    if response.status == 410 {
        return GithubError::Forbidden("les issues sont désactivées pour ce dépôt".to_owned());
    }
    interpret_failure(response)
}

async fn get_json(client: &ApiClient, url: &str) -> Result<Value, GithubError> {
    let response = client.get(url).await?;
    if !response.ok() {
        return Err(failure(&response));
    }
    response.json().map_err(|_| GithubError::Unreadable)
}

async fn github_pages(
    client: &ApiClient,
    first: String,
) -> Result<(Vec<Value>, bool), GithubError> {
    let mut items = Vec::new();
    let mut next = Some(first);
    let mut fetched = 0;
    while let Some(url) = next.take() {
        if fetched == MAX_SEARCH_PAGES {
            return Ok((items, true));
        }
        fetched += 1;
        let response = client.get(&url).await?;
        if !response.ok() {
            return Err(failure(&response));
        }
        let value = response.json().map_err(|_| GithubError::Unreadable)?;
        match value {
            Value::Array(page) => items.extend(page),
            Value::Object(_) => items.extend(
                value
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or(GithubError::Unreadable)?,
            ),
            _ => return Err(GithubError::Unreadable),
        }
        next = response.header("link").and_then(next_link);
    }
    Ok((items, false))
}

pub async fn list_issues(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    filter: IssueFilter,
    query: &str,
) -> Result<(Vec<IssueSummary>, bool), GithubError> {
    let query = query.trim();
    let client = client(store, connection).await?;
    match connection.provider {
        Provider::Github => {
            if !valid_org(owner) || !valid_org(name) {
                return Err(GithubError::NotFound);
            }
            let state = match filter {
                IssueFilter::Open => "open",
                IssueFilter::Closed => "closed",
                IssueFilter::All => "all",
            };
            let (raw, truncated) = if query.is_empty() {
                github_pages(
                    &client,
                    format!("repos/{owner}/{name}/issues?state={state}&sort=updated&per_page=100"),
                )
                .await?
            } else {
                let scope = match filter {
                    IssueFilter::Open => "+is:open",
                    IssueFilter::Closed => "+is:closed",
                    IssueFilter::All => "",
                };
                let (items, cut) = github_pages(
                    &client,
                    format!(
                        "search/issues?q={}+repo:{owner}/{name}+is:issue{scope}&per_page=100",
                        encode(query)
                    ),
                )
                .await?;
                (items, cut)
            };
            Ok((raw.iter().filter_map(github_summary).collect(), truncated))
        }
        Provider::Gitlab => {
            if !crate::gitlab_repos::valid_namespace(owner) || !valid_org(name) {
                return Err(GithubError::NotFound);
            }
            let state = match filter {
                IssueFilter::Open => "&state=opened",
                IssueFilter::Closed => "&state=closed",
                IssueFilter::All => "",
            };
            let search = if query.is_empty() {
                String::new()
            } else {
                format!("&search={}", encode(query))
            };
            let raw = pages(
                &client,
                format!(
                    "projects/{}/issues?order_by=updated_at&per_page=100{state}{search}",
                    encode(&format!("{owner}/{name}"))
                ),
            )
            .await?;
            Ok((
                raw.items.iter().filter_map(gitlab_summary).collect(),
                raw.truncated,
            ))
        }
    }
}

fn note_of(value: &Value, author_field: &str, author_key: &str) -> IssueNote {
    IssueNote {
        author: user(value, author_key, author_field),
        body: text(value, "body").unwrap_or_default(),
        created_at: text(value, "created_at").unwrap_or_default(),
    }
}

fn push_commit(links: &mut Vec<IssueLink>, sha: String) {
    if !links
        .iter()
        .any(|l| matches!(l, IssueLink::Commit { sha: s } if *s == sha))
    {
        links.push(IssueLink::Commit { sha });
    }
}

fn push_pull(links: &mut Vec<IssueLink>, number: u64, title: String, open: bool) {
    if !links
        .iter()
        .any(|l| matches!(l, IssueLink::Pull { number: n, .. } if *n == number))
    {
        links.push(IssueLink::Pull {
            number,
            title,
            open,
        });
    }
}

pub async fn load_issue(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u64,
) -> Result<IssueDetail, GithubError> {
    let client = client(store, connection).await?;
    match connection.provider {
        Provider::Github => load_github(&client, owner, name, number).await,
        Provider::Gitlab => load_gitlab(&client, owner, name, number).await,
    }
}

async fn load_github(
    client: &ApiClient,
    owner: &str,
    name: &str,
    number: u64,
) -> Result<IssueDetail, GithubError> {
    if !valid_org(owner) || !valid_org(name) {
        return Err(GithubError::NotFound);
    }
    let base = format!("repos/{owner}/{name}/issues/{number}");
    let value = get_json(client, &base).await?;
    let summary = github_summary(&value).ok_or(GithubError::NotFound)?;
    let (comments, cut) = github_pages(client, format!("{base}/comments?per_page=100")).await?;
    let mut limits = Vec::new();
    let mut links = Vec::new();
    match github_pages(client, format!("{base}/timeline?per_page=100")).await {
        Ok((events, _)) => {
            for event in &events {
                match text(event, "event").as_deref() {
                    Some("cross-referenced") => {
                        if let Some(issue) = event.get("source").and_then(|s| s.get("issue"))
                            && issue.get("pull_request").is_some()
                            && let Some(n) = issue.get("number").and_then(Value::as_u64)
                        {
                            push_pull(
                                &mut links,
                                n,
                                text(issue, "title").unwrap_or_default(),
                                text(issue, "state").as_deref() == Some("open"),
                            );
                        }
                    }
                    Some("referenced" | "closed") => {
                        if let Some(sha) = text(event, "commit_id") {
                            push_commit(&mut links, sha);
                        }
                    }
                    _ => {}
                }
            }
        }
        Err(GithubError::NotFound | GithubError::Forbidden(_)) => limits.push(
            "l'historique des liens (pull requests et commits) n'est pas lisible avec ce compte"
                .to_owned(),
        ),
        Err(other) => return Err(other),
    }
    Ok(IssueDetail {
        summary,
        body: text(&value, "body").unwrap_or_default(),
        notes: comments
            .iter()
            .map(|c| note_of(c, "login", "user"))
            .collect(),
        notes_truncated: cut,
        links,
        instance: None,
        limits,
    })
}

fn sha_in(body: &str) -> Option<String> {
    let rest = body.split("mentioned in commit ").nth(1)?;
    let sha: String = rest
        .chars()
        .skip_while(|c| !c.is_ascii_hexdigit())
        .take_while(|c| c.is_ascii_hexdigit())
        .collect();
    (sha.len() >= 7).then_some(sha)
}

async fn load_gitlab(
    client: &ApiClient,
    owner: &str,
    name: &str,
    number: u64,
) -> Result<IssueDetail, GithubError> {
    if !crate::gitlab_repos::valid_namespace(owner) || !valid_org(name) {
        return Err(GithubError::NotFound);
    }
    let base = format!(
        "projects/{}/issues/{number}",
        encode(&format!("{owner}/{name}"))
    );
    let value = get_json(client, &base).await?;
    let summary = gitlab_summary(&value).ok_or(GithubError::Unreadable)?;
    let raw = pages(client, format!("{base}/notes?sort=asc&per_page=100")).await?;
    let mut links = Vec::new();
    let mut notes = Vec::new();
    for note in &raw.items {
        if note.get("system").and_then(Value::as_bool) == Some(true) {
            if let Some(sha) = sha_in(&text(note, "body").unwrap_or_default()) {
                push_commit(&mut links, sha);
            }
        } else {
            notes.push(note_of(note, "username", "author"));
        }
    }
    let mut limits = Vec::new();
    for route in ["closed_by", "related_merge_requests"] {
        let response = client.get(&format!("{base}/{route}")).await?;
        if response.status == 404 {
            limits.push(format!(
                "cette instance ne fournit pas « {} »",
                if route == "closed_by" {
                    "merge requests qui ferment l'issue"
                } else {
                    "merge requests liées"
                }
            ));
            continue;
        }
        if !response.ok() {
            return Err(failure(&response));
        }
        if let Value::Array(list) = response.json().map_err(|_| GithubError::Unreadable)? {
            for mr in &list {
                if let Some(iid) = mr.get("iid").and_then(Value::as_u64) {
                    push_pull(
                        &mut links,
                        iid,
                        text(mr, "title").unwrap_or_default(),
                        text(mr, "state").as_deref() == Some("opened"),
                    );
                }
            }
        }
    }
    let version = client.get("version").await?;
    let instance = version
        .ok()
        .then(|| version.json().ok())
        .flatten()
        .and_then(|v| text(&v, "version"))
        .map(|v| format!("GitLab {v}"));
    Ok(IssueDetail {
        summary,
        body: text(&value, "description").unwrap_or_default(),
        notes,
        notes_truncated: raw.truncated || raw.items.len() > MAX_NOTES,
        links,
        instance,
        limits,
    })
}
