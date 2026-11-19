use gitpoulpe_core::{Connection, NoticeKind, Provider};
use serde_json::Value;

use crate::github::GithubError;
use crate::repos::{client, pages};
use crate::secrets::SecretStore;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawNotice {
    pub id: String,
    pub kind: NoticeKind,
    pub title: String,
    pub repo: Option<String>,
    pub number: Option<u64>,
    pub at: String,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn github_number(subject: &Value) -> Option<u64> {
    let url = text(subject, "url")?;
    let (head, tail) = url.rsplit_once('/')?;
    let kind = head.rsplit('/').next()?;
    matches!(kind, "pulls" | "issues")
        .then(|| tail.parse().ok())
        .flatten()
}

fn parse_github(item: &Value) -> Option<RawNotice> {
    if item.get("unread").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let subject = item.get("subject")?;
    let kind = match text(item, "reason").as_deref() {
        Some("review_requested") => NoticeKind::Review,
        Some("mention" | "team_mention") => NoticeKind::Mention,
        Some("ci_activity") => NoticeKind::Checks,
        _ => NoticeKind::Activity,
    };
    Some(RawNotice {
        id: text(item, "id")?,
        kind,
        title: text(subject, "title").unwrap_or_default(),
        repo: item.get("repository").and_then(|r| text(r, "full_name")),
        number: github_number(subject),
        at: text(item, "updated_at").unwrap_or_default(),
    })
}

fn parse_gitlab(item: &Value) -> Option<RawNotice> {
    let kind = match text(item, "action_name").as_deref() {
        Some("review_requested" | "approval_required") => NoticeKind::Review,
        Some("mentioned" | "directly_addressed") => NoticeKind::Mention,
        Some("build_failed") => NoticeKind::Checks,
        _ => NoticeKind::Activity,
    };
    let target = item.get("target");
    Some(RawNotice {
        id: item.get("id")?.to_string(),
        kind,
        title: target.and_then(|t| text(t, "title")).unwrap_or_default(),
        repo: item
            .get("project")
            .and_then(|p| text(p, "path_with_namespace")),
        number: target.and_then(|t| t.get("iid")).and_then(Value::as_u64),
        at: text(item, "updated_at")
            .or_else(|| text(item, "created_at"))
            .unwrap_or_default(),
    })
}

pub async fn fetch_notifications(
    store: &dyn SecretStore,
    connection: &Connection,
) -> Result<Vec<RawNotice>, GithubError> {
    let client = client(store, connection)?;
    match connection.provider {
        Provider::Github => {
            let raw = pages(&client, "notifications?per_page=50".to_owned()).await?;
            Ok(raw.items.iter().filter_map(parse_github).collect())
        }
        Provider::Gitlab => {
            let raw = pages(&client, "todos?state=pending&per_page=50".to_owned()).await?;
            Ok(raw.items.iter().filter_map(parse_gitlab).collect())
        }
    }
}
