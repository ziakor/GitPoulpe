use gitpoulpe_core::Connection;
use serde_json::{Value, json};

use crate::github::{GithubError, interpret_failure};
use crate::pull_comments::{Side, current_head};
use crate::repos::{client, pages, valid_org};
use crate::secrets::SecretStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewDecision {
    Comment,
    Approve,
    RequestChanges,
}

impl ReviewDecision {
    fn api(self) -> &'static str {
        match self {
            Self::Comment => "COMMENT",
            Self::Approve => "APPROVE",
            Self::RequestChanges => "REQUEST_CHANGES",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftLine {
    pub path: String,
    pub line: u64,
    pub start_line: Option<u64>,
    pub side: Side,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewSubmission {
    pub commit_sha: String,
    pub body: String,
    pub decision: ReviewDecision,
    pub lines: Vec<DraftLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub id: u64,
    pub author: String,
    pub state: String,
    pub body: String,
    pub commit_sha: String,
    pub submitted_at: String,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn parse_review(value: &Value) -> Option<Review> {
    Some(Review {
        id: value.get("id")?.as_u64()?,
        author: value
            .get("user")
            .and_then(|u| text(u, "login"))
            .unwrap_or_default(),
        state: text(value, "state").unwrap_or_default(),
        body: text(value, "body").unwrap_or_default(),
        commit_sha: text(value, "commit_id").unwrap_or_default(),
        submitted_at: text(value, "submitted_at").unwrap_or_default(),
    })
}

pub async fn list_reviews(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u64,
) -> Result<Vec<Review>, GithubError> {
    if connection.provider == gitpoulpe_core::Provider::Gitlab {
        let client = client(store, connection)?;
        return crate::gitlab_review::list_reviews(&client, (owner, name), number).await;
    }
    if !valid_org(owner) || !valid_org(name) {
        return Err(GithubError::NotFound);
    }
    let client = client(store, connection)?;
    let raw = pages(
        &client,
        format!("repos/{owner}/{name}/pulls/{number}/reviews?per_page=100"),
    )
    .await?;
    Ok(raw.items.iter().filter_map(parse_review).collect())
}

pub fn review_exists(reviews: &[Review], login: &str, submission: &ReviewSubmission) -> bool {
    let state = match submission.decision {
        ReviewDecision::Comment => "COMMENTED",
        ReviewDecision::Approve => "APPROVED",
        ReviewDecision::RequestChanges => "CHANGES_REQUESTED",
    };
    reviews.iter().any(|r| {
        r.state == state
            && r.author.eq_ignore_ascii_case(login)
            && r.commit_sha == submission.commit_sha
            && r.body.trim() == submission.body.trim()
    })
}

pub async fn submit_review(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u64,
    submission: &ReviewSubmission,
) -> Result<Review, GithubError> {
    if connection.provider == gitpoulpe_core::Provider::Gitlab {
        let client = client(store, connection)?;
        return crate::gitlab_review::submit_review(
            &client,
            connection,
            (owner, name),
            number,
            submission,
        )
        .await;
    }
    if !valid_org(owner) || !valid_org(name) {
        return Err(GithubError::NotFound);
    }
    let body = submission.body.trim();
    if submission.decision == ReviewDecision::RequestChanges && body.is_empty() {
        return Err(GithubError::Validation(
            "expliquez les changements demandés dans le message de la revue".to_owned(),
        ));
    }
    if submission.decision == ReviewDecision::Comment
        && body.is_empty()
        && submission.lines.is_empty()
    {
        return Err(GithubError::Validation(
            "une revue sans message ni commentaire n'a rien à publier".to_owned(),
        ));
    }
    for line in &submission.lines {
        if line.body.trim().is_empty()
            || line.line == 0
            || line.start_line.is_some_and(|s| s == 0 || s > line.line)
        {
            return Err(GithubError::Validation(format!(
                "commentaire invalide sur {}",
                line.path
            )));
        }
    }
    let head = current_head(store, connection, owner, name, number).await?;
    if head != submission.commit_sha {
        return Err(GithubError::AnchorStale {
            prepared: submission.commit_sha.chars().take(7).collect(),
            current: head.chars().take(7).collect(),
        });
    }
    let comments: Vec<Value> = submission
        .lines
        .iter()
        .map(|l| {
            let mut c = json!({
                "path": l.path,
                "line": l.line,
                "side": l.side.api(),
                "body": l.body.trim(),
            });
            if let Some(start) = l.start_line.filter(|s| *s != l.line) {
                c["start_line"] = json!(start);
                c["start_side"] = json!(l.side.api());
            }
            c
        })
        .collect();
    let client = client(store, connection)?;
    let response = client
        .send(
            "POST",
            &format!("repos/{owner}/{name}/pulls/{number}/reviews"),
            Some(json!({
                "commit_id": submission.commit_sha,
                "body": body,
                "event": submission.decision.api(),
                "comments": comments,
            })),
        )
        .await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    parse_review(&response.json().map_err(|_| GithubError::Unreadable)?)
        .ok_or(GithubError::Unreadable)
}
