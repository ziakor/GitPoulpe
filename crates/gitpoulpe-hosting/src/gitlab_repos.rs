use gitpoulpe_core::Connection;
use serde_json::Value;

use crate::github::GithubError;
use crate::http::ApiClient;
use crate::repos::{Listing, Organization, PAGE_SIZE, RepoSummary, pages};

const DEVELOPER: u64 = 30;

pub(crate) fn valid_namespace(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 255
        && path.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
}

pub(crate) fn project_id(value: &Value) -> Option<u64> {
    value.get("id")?.as_u64()
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(crate) async fn list_groups(client: &ApiClient) -> Result<Listing<Organization>, GithubError> {
    let raw = pages(
        client,
        format!("groups?min_access_level=10&order_by=path&per_page={PAGE_SIZE}"),
    )
    .await?;
    let mut items: Vec<Organization> = raw
        .items
        .iter()
        .filter_map(|g| {
            Some(Organization {
                login: text(g, "full_path")?,
                description: text(g, "description"),
            })
        })
        .collect();
    items.sort_by_key(|g| g.login.to_lowercase());
    Ok(Listing {
        items,
        truncated: raw.truncated,
    })
}

pub(crate) fn parse_project(connection: &Connection, value: &Value) -> Option<RepoSummary> {
    let name = text(value, "path")?;
    let owner = value.get("namespace").and_then(|n| text(n, "full_path"))?;
    let clone_url = text(value, "http_url_to_repo")?;
    let access = |key: &str| {
        value
            .get("permissions")
            .and_then(|p| p.get(key))
            .and_then(|a| a.get("access_level"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let level = access("project_access").max(access("group_access"));
    Some(RepoSummary {
        connection_id: connection.id.clone(),
        owner,
        name,
        description: text(value, "description"),
        private: text(value, "visibility").as_deref() != Some("public"),
        fork: value
            .get("forked_from_project")
            .is_some_and(|f| !f.is_null()),
        archived: value
            .get("archived")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        default_branch: text(value, "default_branch"),
        clone_url,
        parent: value
            .get("forked_from_project")
            .and_then(|f| text(f, "path_with_namespace")),
        can_push: level >= DEVELOPER,
    })
}

pub(crate) async fn list_projects(
    connection: &Connection,
    client: &ApiClient,
    namespace: Option<&str>,
) -> Result<Listing<RepoSummary>, GithubError> {
    let first = match namespace {
        Some(path) if !valid_namespace(path) => return Err(GithubError::NotFound),
        Some(path) => format!(
            "groups/{}/projects?include_subgroups=true&order_by=path&per_page={PAGE_SIZE}",
            path.replace('/', "%2F")
        ),
        None => format!("projects?membership=true&order_by=path&per_page={PAGE_SIZE}"),
    };
    let raw = pages(client, first).await?;
    let mut items: Vec<RepoSummary> = raw
        .items
        .iter()
        .filter_map(|p| parse_project(connection, p))
        .collect();
    items.sort_by_key(|r| r.full_name().to_lowercase());
    Ok(Listing {
        items,
        truncated: raw.truncated,
    })
}
