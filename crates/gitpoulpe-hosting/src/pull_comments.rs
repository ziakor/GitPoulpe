use gitpoulpe_core::Connection;
use serde_json::{Value, json};

use crate::attachments::{Attachment, github_attachments};
use crate::github::{GithubError, interpret_failure};
use crate::repos::{client, pages, valid_org};

pub(crate) const RICH: &str = "application/vnd.github.full+json";
use crate::secrets::SecretStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn api(self) -> &'static str {
        match self {
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineAnchor {
    pub commit_sha: String,
    pub path: String,
    pub line: u64,
    pub start_line: Option<u64>,
    pub side: Side,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueComment {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub attachments: Vec<Attachment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewComment {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub path: String,
    pub line: Option<u64>,
    pub side: Side,
    pub commit_sha: String,
    pub in_reply_to: Option<u64>,
    pub outdated: bool,
    pub created_at: String,
    pub attachments: Vec<Attachment>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PullComments {
    pub issue: Vec<IssueComment>,
    pub review: Vec<ReviewComment>,
}

impl PullComments {
    pub fn contains(&self, login: &str, body: &str) -> bool {
        let body = body.trim();
        self.issue
            .iter()
            .any(|c| c.author.eq_ignore_ascii_case(login) && c.body.trim() == body)
            || self
                .review
                .iter()
                .any(|c| c.author.eq_ignore_ascii_case(login) && c.body.trim() == body)
    }
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn author(value: &Value) -> String {
    value
        .get("user")
        .and_then(|u| text(u, "login"))
        .unwrap_or_default()
}

fn attachments_of(value: &Value, web: &str) -> Vec<Attachment> {
    github_attachments(
        value.get("body").and_then(Value::as_str).unwrap_or(""),
        value.get("body_html").and_then(Value::as_str),
        web,
    )
}

fn parse_issue(value: &Value, web: &str) -> Option<IssueComment> {
    Some(IssueComment {
        attachments: attachments_of(value, web),
        id: value.get("id")?.as_u64()?,
        author: author(value),
        body: text(value, "body").unwrap_or_default(),
        created_at: text(value, "created_at").unwrap_or_default(),
    })
}

fn parse_review(value: &Value, web: &str) -> Option<ReviewComment> {
    let line = value.get("line").and_then(Value::as_u64);
    let original = value.get("original_line").and_then(Value::as_u64);
    Some(ReviewComment {
        attachments: attachments_of(value, web),
        id: value.get("id")?.as_u64()?,
        author: author(value),
        body: text(value, "body").unwrap_or_default(),
        path: text(value, "path")?,
        outdated: line.is_none(),
        line: line.or(original),
        side: if text(value, "side").as_deref() == Some("LEFT") {
            Side::Left
        } else {
            Side::Right
        },
        commit_sha: text(value, "commit_id").unwrap_or_default(),
        in_reply_to: value.get("in_reply_to_id").and_then(Value::as_u64),
        created_at: text(value, "created_at").unwrap_or_default(),
    })
}

fn checked(owner: &str, name: &str) -> Result<(), GithubError> {
    if valid_org(owner) && valid_org(name) {
        Ok(())
    } else {
        Err(GithubError::NotFound)
    }
}

fn nonempty(body: &str) -> Result<&str, GithubError> {
    let body = body.trim();
    if body.is_empty() {
        Err(GithubError::Validation(
            "le commentaire est vide".to_owned(),
        ))
    } else {
        Ok(body)
    }
}

pub async fn list_comments(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u64,
) -> Result<PullComments, GithubError> {
    if connection.provider == gitpoulpe_core::Provider::Gitlab {
        let client = client(store, connection).await?;
        return crate::gitlab_review::list_comments(&client, (owner, name), number).await;
    }
    checked(owner, name)?;
    let client = client(store, connection).await?.with_accept(RICH);
    let issue = pages(
        &client,
        format!("repos/{owner}/{name}/issues/{number}/comments?per_page=100"),
    )
    .await?;
    let review = pages(
        &client,
        format!("repos/{owner}/{name}/pulls/{number}/comments?per_page=100"),
    )
    .await?;
    Ok(PullComments {
        issue: issue
            .items
            .iter()
            .filter_map(|c| parse_issue(c, &connection.web))
            .collect(),
        review: review
            .items
            .iter()
            .filter_map(|c| parse_review(c, &connection.web))
            .collect(),
    })
}

pub async fn post_comment(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u64,
    body: &str,
) -> Result<IssueComment, GithubError> {
    if connection.provider == gitpoulpe_core::Provider::Gitlab {
        let client = client(store, connection).await?;
        return crate::gitlab_review::post_comment(&client, (owner, name), number, body).await;
    }
    checked(owner, name)?;
    let body = nonempty(body)?;
    let client = client(store, connection).await?;
    let response = client
        .send(
            "POST",
            &format!("repos/{owner}/{name}/issues/{number}/comments"),
            Some(json!({ "body": body })),
        )
        .await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    parse_issue(
        &response.json().map_err(|_| GithubError::Unreadable)?,
        &connection.web,
    )
    .ok_or(GithubError::Unreadable)
}

pub async fn current_head(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u64,
) -> Result<String, GithubError> {
    if connection.provider == gitpoulpe_core::Provider::Gitlab {
        let client = client(store, connection).await?;
        return crate::gitlab_review::current_head(&client, (owner, name), number).await;
    }
    checked(owner, name)?;
    let client = client(store, connection).await?;
    let response = client
        .get(&format!("repos/{owner}/{name}/pulls/{number}"))
        .await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    response
        .json()
        .ok()
        .and_then(|v| v.get("head")?.get("sha")?.as_str().map(str::to_owned))
        .ok_or(GithubError::Unreadable)
}

pub async fn post_line_comment(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u64,
    anchor: &LineAnchor,
    body: &str,
) -> Result<ReviewComment, GithubError> {
    if connection.provider == gitpoulpe_core::Provider::Gitlab {
        let client = client(store, connection).await?;
        return crate::gitlab_review::post_line_comment(
            &client,
            (owner, name),
            number,
            anchor,
            body,
        )
        .await;
    }
    checked(owner, name)?;
    let body = nonempty(body)?;
    if anchor.line == 0 || anchor.start_line.is_some_and(|s| s == 0 || s > anchor.line) {
        return Err(GithubError::Validation(
            "plage de lignes invalide".to_owned(),
        ));
    }
    let head = current_head(store, connection, owner, name, number).await?;
    if head != anchor.commit_sha {
        return Err(GithubError::AnchorStale {
            prepared: anchor.commit_sha.chars().take(7).collect(),
            current: head.chars().take(7).collect(),
        });
    }
    let client = client(store, connection).await?;
    let mut payload = json!({
        "body": body,
        "commit_id": anchor.commit_sha,
        "path": anchor.path,
        "line": anchor.line,
        "side": anchor.side.api(),
    });
    if let Some(start) = anchor.start_line.filter(|s| *s != anchor.line) {
        payload["start_line"] = json!(start);
        payload["start_side"] = json!(anchor.side.api());
    }
    let response = client
        .send(
            "POST",
            &format!("repos/{owner}/{name}/pulls/{number}/comments"),
            Some(payload),
        )
        .await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    parse_review(
        &response.json().map_err(|_| GithubError::Unreadable)?,
        &connection.web,
    )
    .ok_or(GithubError::Unreadable)
}

pub async fn reply_to_comment(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u64,
    comment_id: u64,
    body: &str,
) -> Result<ReviewComment, GithubError> {
    if connection.provider == gitpoulpe_core::Provider::Gitlab {
        let client = client(store, connection).await?;
        return crate::gitlab_review::reply_to_comment(
            &client,
            (owner, name),
            number,
            comment_id,
            body,
        )
        .await;
    }
    checked(owner, name)?;
    let body = nonempty(body)?;
    let client = client(store, connection).await?;
    let response = client
        .send(
            "POST",
            &format!("repos/{owner}/{name}/pulls/{number}/comments/{comment_id}/replies"),
            Some(json!({ "body": body })),
        )
        .await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    parse_review(
        &response.json().map_err(|_| GithubError::Unreadable)?,
        &connection.web,
    )
    .ok_or(GithubError::Unreadable)
}
