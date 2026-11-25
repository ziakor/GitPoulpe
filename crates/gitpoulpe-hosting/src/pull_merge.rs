use gitpoulpe_core::Connection;
use serde_json::{Value, json};

use crate::checks::{CheckState, load_checks};
use crate::github::{GithubError, interpret_failure};
use crate::pull_comments::current_head;
use crate::pull_read::PullState;
use crate::pull_review::list_reviews;
use crate::repos::{client, valid_org};
use crate::secrets::SecretStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    fn api(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequiredCheck {
    pub name: String,
    pub state: Option<CheckState>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeReadiness {
    pub head_sha: String,
    pub state: PullState,
    pub draft: bool,
    pub mergeable_state: String,
    pub methods: Vec<MergeMethod>,
    pub required_checks: Vec<RequiredCheck>,
    pub required_approvals: u64,
    pub approvals: u64,
    pub protection_unreadable: bool,
    pub blockers: Vec<String>,
}

impl MergeReadiness {
    pub fn can_attempt(&self) -> bool {
        self.state == PullState::Open && !self.draft && !self.methods.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeRequest {
    pub method: MergeMethod,
    pub expected_head: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostedMerge {
    pub sha: String,
    pub verified_merged: bool,
}

pub(crate) fn missing_approvals(approvals: u64, required: u64) -> String {
    format!(
        "{approvals} {} sur {required} {}",
        if approvals > 1 {
            "approbations"
        } else {
            "approbation"
        },
        if required > 1 { "requises" } else { "requise" }
    )
}

fn explain(state: &str) -> Option<&'static str> {
    match state {
        "blocked" => Some(
            "bloquée par les protections de branche (revues ou vérifications requises manquantes)",
        ),
        "behind" => Some("la branche de la pull request est en retard sur la branche cible"),
        "dirty" => Some("des conflits de fusion doivent être résolus"),
        "unstable" => Some("des vérifications non obligatoires échouent"),
        "draft" => Some("la pull request est un brouillon"),
        "has_hooks" | "clean" | "unknown" | "" => None,
        _ => Some("état de fusion non reconnu"),
    }
}

pub async fn prepare_merge(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u64,
) -> Result<MergeReadiness, GithubError> {
    if connection.provider == gitpoulpe_core::Provider::Gitlab {
        let client = client(store, connection).await?;
        return crate::gitlab_merge::prepare_merge(&client, (owner, name), number).await;
    }
    if !valid_org(owner) || !valid_org(name) {
        return Err(GithubError::NotFound);
    }
    let client = client(store, connection).await?;
    let get = |path: String| {
        let client = client.clone();
        async move { client.get(&path).await }
    };
    let repo = get(format!("repos/{owner}/{name}")).await?;
    if !repo.ok() {
        return Err(interpret_failure(&repo));
    }
    let repo = repo.json().map_err(|_| GithubError::Unreadable)?;
    let allowed = |key: &str| repo.get(key).and_then(Value::as_bool).unwrap_or(false);
    let mut methods = Vec::new();
    if allowed("allow_merge_commit") {
        methods.push(MergeMethod::Merge);
    }
    if allowed("allow_squash_merge") {
        methods.push(MergeMethod::Squash);
    }
    if allowed("allow_rebase_merge") {
        methods.push(MergeMethod::Rebase);
    }
    let pull = get(format!("repos/{owner}/{name}/pulls/{number}")).await?;
    if !pull.ok() {
        return Err(interpret_failure(&pull));
    }
    let pull = pull.json().map_err(|_| GithubError::Unreadable)?;
    let head_sha = pull
        .get("head")
        .and_then(|h| h.get("sha"))
        .and_then(Value::as_str)
        .ok_or(GithubError::Unreadable)?
        .to_owned();
    let base_ref = pull
        .get("base")
        .and_then(|b| b.get("ref"))
        .and_then(Value::as_str)
        .ok_or(GithubError::Unreadable)?
        .to_owned();
    let merged = pull.get("merged_at").is_some_and(|m| !m.is_null());
    let state = match (pull.get("state").and_then(Value::as_str), merged) {
        (_, true) => PullState::Merged,
        (Some("open"), false) => PullState::Open,
        _ => PullState::Closed,
    };
    let draft = pull.get("draft").and_then(Value::as_bool).unwrap_or(false);
    let mergeable_state = pull
        .get("mergeable_state")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let protection = get(format!(
        "repos/{owner}/{name}/branches/{base_ref}/protection"
    ))
    .await?;
    let (contexts, required_approvals, protection_unreadable) = match protection.status {
        404 => (Vec::new(), 0, false),
        403 => (Vec::new(), 0, true),
        _ if protection.ok() => {
            let value = protection.json().map_err(|_| GithubError::Unreadable)?;
            let contexts = value
                .get("required_status_checks")
                .and_then(|r| r.get("contexts"))
                .and_then(Value::as_array)
                .map(|c| {
                    c.iter()
                        .filter_map(|n| n.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let approvals = value
                .get("required_pull_request_reviews")
                .and_then(|r| r.get("required_approving_review_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            (contexts, approvals, false)
        }
        _ => return Err(interpret_failure(&protection)),
    };
    let checks = load_checks(store, connection, owner, name, number, &head_sha).await?;
    let required_checks: Vec<RequiredCheck> = contexts
        .into_iter()
        .map(|context| RequiredCheck {
            state: checks
                .items
                .iter()
                .find(|c| c.name == context)
                .map(|c| c.state),
            name: context,
        })
        .collect();
    let reviews = list_reviews(store, connection, owner, name, number).await?;
    let mut latest: Vec<(&str, &str)> = Vec::new();
    for review in reviews
        .iter()
        .filter(|r| r.state != "COMMENTED" && r.state != "PENDING")
    {
        latest.retain(|(author, _)| *author != review.author);
        latest.push((&review.author, &review.state));
    }
    let approvals = latest.iter().filter(|(_, s)| *s == "APPROVED").count() as u64;
    let changes = latest.iter().any(|(_, s)| *s == "CHANGES_REQUESTED");
    let mut blockers = Vec::new();
    if state != PullState::Open {
        blockers.push("la pull request n'est plus ouverte".to_owned());
    }
    if draft {
        blockers.push("la pull request est un brouillon".to_owned());
    }
    if methods.is_empty() {
        blockers.push("le dépôt n'autorise aucune méthode de fusion".to_owned());
    }
    if let Some(why) = explain(&mergeable_state) {
        blockers.push(why.to_owned());
    }
    for check in &required_checks {
        match check.state {
            Some(CheckState::Success) | Some(CheckState::Skipped) => {}
            Some(CheckState::Pending) => {
                blockers.push(format!(
                    "la vérification requise « {} » est en attente",
                    check.name
                ));
            }
            Some(_) => blockers.push(format!(
                "la vérification requise « {} » n'a pas réussi",
                check.name
            )),
            None => blockers.push(format!(
                "la vérification requise « {} » est absente",
                check.name
            )),
        }
    }
    if approvals < required_approvals {
        blockers.push(missing_approvals(approvals, required_approvals));
    }
    if changes {
        blockers.push("des changements ont été demandés par un relecteur".to_owned());
    }
    Ok(MergeReadiness {
        head_sha,
        state,
        draft,
        mergeable_state,
        methods,
        required_checks,
        required_approvals,
        approvals,
        protection_unreadable,
        blockers,
    })
}

pub async fn merge_pull(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u64,
    request: &MergeRequest,
) -> Result<HostedMerge, GithubError> {
    if connection.provider == gitpoulpe_core::Provider::Gitlab {
        let client = client(store, connection).await?;
        return crate::gitlab_merge::merge_pull(
            &client,
            connection,
            (owner, name),
            number,
            request,
        )
        .await;
    }
    if !valid_org(owner) || !valid_org(name) {
        return Err(GithubError::NotFound);
    }
    let head = current_head(store, connection, owner, name, number).await?;
    if head != request.expected_head {
        return Err(GithubError::AnchorStale {
            prepared: request.expected_head.chars().take(7).collect(),
            current: head.chars().take(7).collect(),
        });
    }
    let client = client(store, connection).await?;
    let mut body = json!({ "merge_method": request.method.api(), "sha": request.expected_head });
    if !request.message.trim().is_empty() {
        body["commit_message"] = json!(request.message.trim());
    }
    let response = client
        .send(
            "PUT",
            &format!("repos/{owner}/{name}/pulls/{number}/merge"),
            Some(body),
        )
        .await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    let value = response.json().map_err(|_| GithubError::Unreadable)?;
    if value.get("merged").and_then(Value::as_bool) != Some(true) {
        return Err(GithubError::MergeRefused(
            value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("la fusion n'a pas été effectuée")
                .to_owned(),
        ));
    }
    let sha = value
        .get("sha")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let verified_merged = is_merged(store, connection, owner, name, number).await?;
    Ok(HostedMerge {
        sha,
        verified_merged,
    })
}

pub async fn is_merged(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u64,
) -> Result<bool, GithubError> {
    let client = client(store, connection).await?;
    if connection.provider == gitpoulpe_core::Provider::Gitlab {
        return crate::gitlab_merge::is_merged(&client, (owner, name), number).await;
    }
    let response = client
        .get(&format!("repos/{owner}/{name}/pulls/{number}"))
        .await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    let value = response.json().map_err(|_| GithubError::Unreadable)?;
    Ok(value.get("merged_at").is_some_and(|m| !m.is_null())
        || value.get("merged").and_then(Value::as_bool) == Some(true))
}
