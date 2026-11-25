use gitpoulpe_core::Connection;
use serde_json::Value;

use crate::github::{GithubError, interpret_failure};
use crate::http::next_link;
use crate::repos::{client, valid_org};
use crate::secrets::SecretStore;

const MAX_PAGES: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Pending,
    Success,
    Failure,
    Cancelled,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub state: CheckState,
    pub url: String,
    pub legacy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checks {
    pub sha: String,
    pub items: Vec<Check>,
    pub truncated: bool,
}

impl Checks {
    pub fn count(&self, state: CheckState) -> usize {
        self.items.iter().filter(|c| c.state == state).count()
    }

    pub fn overall(&self) -> Option<CheckState> {
        if self.items.is_empty() {
            None
        } else if self.count(CheckState::Failure) > 0 {
            Some(CheckState::Failure)
        } else if self.count(CheckState::Pending) > 0 {
            Some(CheckState::Pending)
        } else if self.count(CheckState::Cancelled) > 0 {
            Some(CheckState::Cancelled)
        } else if self.count(CheckState::Success) > 0 {
            Some(CheckState::Success)
        } else {
            Some(CheckState::Skipped)
        }
    }
}

fn run_state(run: &Value) -> CheckState {
    if run.get("status").and_then(Value::as_str) != Some("completed") {
        return CheckState::Pending;
    }
    match run.get("conclusion").and_then(Value::as_str) {
        Some("success") => CheckState::Success,
        Some("cancelled") => CheckState::Cancelled,
        Some("skipped" | "neutral") => CheckState::Skipped,
        _ => CheckState::Failure,
    }
}

fn status_state(status: &Value) -> CheckState {
    match status.get("state").and_then(Value::as_str) {
        Some("success") => CheckState::Success,
        Some("pending") => CheckState::Pending,
        _ => CheckState::Failure,
    }
}

pub async fn load_checks(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u64,
    sha: &str,
) -> Result<Checks, GithubError> {
    if connection.provider == gitpoulpe_core::Provider::Gitlab {
        let client = client(store, connection).await?;
        return crate::gitlab_merge::load_checks(&client, (owner, name), number, sha).await;
    }
    let sha_ok =
        !sha.is_empty() && sha.len() <= 64 && sha.chars().all(|c| c.is_ascii_alphanumeric());
    if !valid_org(owner) || !valid_org(name) || !sha_ok {
        return Err(GithubError::NotFound);
    }
    let client = client(store, connection).await?;
    let mut items = Vec::new();
    let mut truncated = false;
    let mut next = Some(format!(
        "repos/{owner}/{name}/commits/{sha}/check-runs?per_page=100"
    ));
    let mut pages = 0;
    while let Some(url) = next.take() {
        if pages == MAX_PAGES {
            truncated = true;
            break;
        }
        pages += 1;
        let response = client.get(&url).await?;
        if !response.ok() {
            return Err(interpret_failure(&response));
        }
        let value = response.json().map_err(|_| GithubError::Unreadable)?;
        let runs = value
            .get("check_runs")
            .and_then(Value::as_array)
            .ok_or(GithubError::Unreadable)?;
        items.extend(runs.iter().filter_map(|run| {
            Some(Check {
                name: run.get("name")?.as_str()?.to_owned(),
                state: run_state(run),
                url: run
                    .get("html_url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                legacy: false,
            })
        }));
        next = response.header("link").and_then(next_link);
    }
    let response = client
        .get(&format!("repos/{owner}/{name}/commits/{sha}/status"))
        .await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    let value = response.json().map_err(|_| GithubError::Unreadable)?;
    if let Some(statuses) = value.get("statuses").and_then(Value::as_array) {
        items.extend(statuses.iter().filter_map(|s| {
            Some(Check {
                name: s.get("context")?.as_str()?.to_owned(),
                state: status_state(s),
                url: s
                    .get("target_url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                legacy: true,
            })
        }));
    }
    Ok(Checks {
        sha: sha.to_owned(),
        items,
        truncated,
    })
}
