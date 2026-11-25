use gitpoulpe_core::Connection;
use serde_json::Value;

use crate::github::{GithubError, interpret_failure};
use crate::http::{ApiClient, next_link};
use crate::secrets::SecretStore;

pub(crate) const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Organization {
    pub login: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoSummary {
    pub connection_id: String,
    pub owner: String,
    pub name: String,
    pub description: Option<String>,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
    pub default_branch: Option<String>,
    pub clone_url: String,
    pub can_push: bool,
    pub parent: Option<String>,
}

impl RepoSummary {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing<T> {
    pub items: Vec<T>,
    pub truncated: bool,
}

pub(crate) async fn client(
    store: &dyn SecretStore,
    connection: &Connection,
) -> Result<ApiClient, GithubError> {
    let token = crate::renewal::access_token(store, connection, None).await?;
    Ok(ApiClient::new(&connection.api, Some(&token))?)
}

pub(crate) async fn pages(
    client: &ApiClient,
    first: String,
) -> Result<Listing<Value>, GithubError> {
    let mut items = Vec::new();
    let mut next = Some(first);
    let mut fetched = 0;
    while let Some(url) = next.take() {
        if fetched == MAX_PAGES {
            return Ok(Listing {
                items,
                truncated: true,
            });
        }
        fetched += 1;
        let response = client.get(&url).await?;
        if !response.ok() {
            return Err(interpret_failure(&response));
        }
        match response.json().map_err(|_| GithubError::Unreadable)? {
            Value::Array(page) => items.extend(page),
            _ => return Err(GithubError::Unreadable),
        }
        next = response.header("link").and_then(next_link);
    }
    Ok(Listing {
        items,
        truncated: false,
    })
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(crate) fn parse_repo(connection: &Connection, value: &Value) -> Option<RepoSummary> {
    let name = text(value, "name")?;
    let owner = text(value.get("owner")?, "login")?;
    let clone_url = text(value, "clone_url")?;
    let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    Some(RepoSummary {
        connection_id: connection.id.clone(),
        owner,
        name,
        description: text(value, "description"),
        private: flag("private"),
        fork: flag("fork"),
        archived: flag("archived"),
        default_branch: text(value, "default_branch"),
        clone_url,
        parent: value.get("parent").and_then(|p| text(p, "full_name")),
        can_push: value
            .get("permissions")
            .and_then(|p| p.get("push"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

pub(crate) fn valid_org(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 100
        && login
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub async fn list_organizations(
    store: &dyn SecretStore,
    connection: &Connection,
) -> Result<Listing<Organization>, GithubError> {
    let client = client(store, connection).await?;
    if connection.provider == gitpoulpe_core::Provider::Gitlab {
        return crate::gitlab_repos::list_groups(&client).await;
    }
    let raw = pages(&client, format!("user/orgs?per_page={PAGE_SIZE}")).await?;
    let mut items: Vec<Organization> = raw
        .items
        .iter()
        .filter_map(|o| {
            Some(Organization {
                login: text(o, "login")?,
                description: text(o, "description"),
            })
        })
        .collect();
    items.sort_by_key(|o| o.login.to_lowercase());
    Ok(Listing {
        items,
        truncated: raw.truncated,
    })
}

pub async fn list_repositories(
    store: &dyn SecretStore,
    connection: &Connection,
    organization: Option<&str>,
) -> Result<Listing<RepoSummary>, GithubError> {
    let client = client(store, connection).await?;
    if connection.provider == gitpoulpe_core::Provider::Gitlab {
        return crate::gitlab_repos::list_projects(connection, &client, organization).await;
    }
    let first = match organization {
        Some(org) if !valid_org(org) => return Err(GithubError::NotFound),
        Some(org) => format!("orgs/{org}/repos?type=all&per_page={PAGE_SIZE}"),
        None => {
            format!("user/repos?affiliation=owner,collaborator&sort=full_name&per_page={PAGE_SIZE}")
        }
    };
    let raw = pages(&client, first).await?;
    let mut items: Vec<RepoSummary> = raw
        .items
        .iter()
        .filter_map(|r| parse_repo(connection, r))
        .collect();
    items.sort_by_key(|r| r.full_name().to_lowercase());
    Ok(Listing {
        items,
        truncated: raw.truncated,
    })
}
