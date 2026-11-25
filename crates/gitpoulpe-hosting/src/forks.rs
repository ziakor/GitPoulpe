use std::future::Future;
use std::time::Duration;

use gitpoulpe_core::Connection;
use serde_json::json;

use crate::github::{GithubError, interpret_failure};
use crate::http::ApiClient;
use crate::repos::{RepoSummary, client, parse_repo, valid_org};
use crate::secrets::SecretStore;

const READY_ATTEMPTS: usize = 10;
const READY_DELAY: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkOutcome {
    pub repo: RepoSummary,
    pub existed: bool,
}

async fn fetch(
    client: &ApiClient,
    connection: &Connection,
    owner: &str,
    name: &str,
) -> Result<Option<RepoSummary>, GithubError> {
    let response = client.get(&format!("repos/{owner}/{name}")).await?;
    match response.status {
        404 => Ok(None),
        _ if !response.ok() => Err(interpret_failure(&response)),
        _ => {
            let value = response.json().map_err(|_| GithubError::Unreadable)?;
            parse_repo(connection, &value)
                .map(Some)
                .ok_or(GithubError::Unreadable)
        }
    }
}

pub async fn create_fork<S, F>(
    store: &dyn SecretStore,
    connection: &Connection,
    source_owner: &str,
    source_name: &str,
    destination: Option<&str>,
    sleep: S,
) -> Result<ForkOutcome, GithubError>
where
    S: Fn(Duration) -> F,
    F: Future<Output = ()>,
{
    if !valid_org(source_owner)
        || !valid_org(source_name)
        || destination.is_some_and(|d| !valid_org(d))
    {
        return Err(GithubError::NotFound);
    }
    let client = client(store, connection).await?;
    let target = destination.unwrap_or(&connection.login).to_owned();
    let source = format!("{source_owner}/{source_name}");
    if let Some(existing) = fetch(&client, connection, &target, source_name).await? {
        return if existing.parent.as_deref() == Some(source.as_str()) {
            Ok(ForkOutcome {
                repo: existing,
                existed: true,
            })
        } else {
            Err(GithubError::NameTaken(existing.full_name()))
        };
    }
    let body = destination.map(|org| json!({ "organization": org }));
    let response = client
        .send(
            "POST",
            &format!("repos/{source_owner}/{source_name}/forks"),
            body.or_else(|| Some(json!({}))),
        )
        .await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    for attempt in 0..READY_ATTEMPTS {
        if let Some(repo) = fetch(&client, connection, &target, source_name).await? {
            return Ok(ForkOutcome {
                repo,
                existed: false,
            });
        }
        if attempt + 1 < READY_ATTEMPTS {
            sleep(READY_DELAY).await;
        }
    }
    Err(GithubError::ForkPending)
}
