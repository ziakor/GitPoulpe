use gitpoulpe_core::Connection;
use serde_json::{Value, json};

use crate::checks::{Check, CheckState, Checks};
use crate::github::{GithubError, interpret_failure};
use crate::gitlab_pulls::{flag, get_json, mr_base, project_path, text, valid_path};
use crate::gitlab_review::{current_head, failure};
use crate::http::ApiClient;
use crate::pull_merge::{HostedMerge, MergeMethod, MergeReadiness, MergeRequest, RequiredCheck};
use crate::pull_read::PullState;
use crate::pulls::encode;
use crate::repos::pages;

type Repo<'a> = (&'a str, &'a str);

fn job_state(status: &str) -> CheckState {
    match status {
        "success" => CheckState::Success,
        "failed" => CheckState::Failure,
        "canceled" | "canceling" => CheckState::Cancelled,
        "skipped" => CheckState::Skipped,
        _ => CheckState::Pending,
    }
}

fn checked(repo: Repo<'_>) -> Result<(), GithubError> {
    if valid_path(repo.0, repo.1) {
        Ok(())
    } else {
        Err(GithubError::NotFound)
    }
}

pub(crate) async fn load_checks(
    client: &ApiClient,
    repo: Repo<'_>,
    number: u64,
    sha: &str,
) -> Result<Checks, GithubError> {
    checked(repo)?;
    let listed = pages(
        client,
        format!("{}/pipelines?per_page=100", mr_base(repo, number)),
    )
    .await?;
    let Some(pipeline) = listed
        .items
        .iter()
        .find(|p| text(p, "sha").as_deref() == Some(sha))
    else {
        return Ok(Checks {
            sha: sha.to_owned(),
            items: Vec::new(),
            truncated: listed.truncated,
        });
    };
    let id = pipeline
        .get("id")
        .and_then(Value::as_u64)
        .ok_or(GithubError::Unreadable)?;
    let project = pipeline
        .get("project_id")
        .and_then(Value::as_u64)
        .ok_or(GithubError::Unreadable)?;
    let jobs = pages(
        client,
        format!("projects/{project}/pipelines/{id}/jobs?per_page=100"),
    )
    .await?;
    let mut items: Vec<Check> = jobs
        .items
        .iter()
        .filter_map(|job| {
            let status = text(job, "status")?;
            let allowed = status == "failed" && flag(job, "allow_failure");
            Some(Check {
                name: format!(
                    "{}{}",
                    text(job, "name")?,
                    if allowed { " (échec autorisé)" } else { "" }
                ),
                state: job_state(&status),
                url: text(job, "web_url").unwrap_or_default(),
                legacy: false,
            })
        })
        .collect();
    if items.is_empty() {
        items.push(Check {
            name: "Pipeline".to_owned(),
            state: job_state(&text(pipeline, "status").unwrap_or_default()),
            url: text(pipeline, "web_url").unwrap_or_default(),
            legacy: false,
        });
    }
    Ok(Checks {
        sha: sha.to_owned(),
        items,
        truncated: listed.truncated || jobs.truncated,
    })
}

fn access_level(project: &Value) -> u64 {
    let level = |key: &str| {
        project
            .get("permissions")
            .and_then(|p| p.get(key))
            .and_then(|a| a.get("access_level"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    level("project_access").max(level("group_access"))
}

fn explain(status: &str) -> Option<String> {
    let message = match status {
        "mergeable" | "not_open" | "draft_status" | "ci_must_pass" | "ci_still_running"
        | "not_approved" => return None,
        "checking" | "unchecked" => {
            "GitLab calcule encore l'état de fusion : actualisez dans un instant"
        }
        "conflict" => "des conflits de fusion doivent être résolus",
        "need_rebase" => "la branche doit être rebasée sur la branche cible",
        "discussions_not_resolved" => "des discussions ne sont pas résolues",
        "requested_changes" => "des changements ont été demandés par un relecteur",
        "external_status_checks" => "des vérifications externes n'ont pas réussi",
        "jira_association_missing" => "une référence Jira est requise",
        "blocked_status" => "bloquée par une autre merge request",
        "locked_paths" => "des fichiers verrouillés par un autre utilisateur sont modifiés",
        other => return Some(format!("état de fusion non reconnu : {other}")),
    };
    Some(message.to_owned())
}

fn pipeline_state(status: &str) -> CheckState {
    match status {
        "success" => CheckState::Success,
        "failed" => CheckState::Failure,
        "canceled" => CheckState::Cancelled,
        "skipped" => CheckState::Skipped,
        _ => CheckState::Pending,
    }
}

pub(crate) async fn prepare_merge(
    client: &ApiClient,
    repo: Repo<'_>,
    number: u64,
) -> Result<MergeReadiness, GithubError> {
    checked(repo)?;
    let project = get_json(
        client,
        &format!("projects/{}", project_path(repo.0, repo.1)),
    )
    .await?;
    let mr = get_json(client, &mr_base(repo, number)).await?;
    let head_sha = text(&mr, "sha").ok_or(GithubError::Unreadable)?;
    let target = text(&mr, "target_branch").ok_or(GithubError::Unreadable)?;
    let state = match text(&mr, "state").as_deref() {
        Some("merged") => PullState::Merged,
        Some("opened" | "locked") => PullState::Open,
        _ => PullState::Closed,
    };
    let draft = flag(&mr, "draft");
    let status = text(&mr, "detailed_merge_status").unwrap_or_default();
    let squash_option = text(&project, "squash_option").unwrap_or_default();
    let mut methods = Vec::new();
    if squash_option != "always" {
        methods.push(if text(&project, "merge_method").as_deref() == Some("ff") {
            MergeMethod::Rebase
        } else {
            MergeMethod::Merge
        });
    }
    if squash_option != "never" {
        methods.push(MergeMethod::Squash);
    }
    let protection = client
        .get(&format!(
            "projects/{}/protected_branches/{}",
            project_path(repo.0, repo.1),
            encode(&target)
        ))
        .await?;
    let user_level = access_level(&project);
    let mut protection_unreadable = false;
    let mut protected_refusal = false;
    match protection.status {
        404 => {}
        403 => protection_unreadable = true,
        _ if protection.ok() => {
            let value = protection.json().map_err(|_| GithubError::Unreadable)?;
            let levels: Vec<u64> = value
                .get("merge_access_levels")
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(|e| e.get("access_level").and_then(Value::as_u64))
                        .collect()
                })
                .unwrap_or_default();
            protected_refusal =
                user_level > 0 && !levels.iter().any(|l| *l > 0 && *l <= user_level);
        }
        _ => return Err(interpret_failure(&protection)),
    }
    let approvals_response = client
        .get(&format!("{}/approvals", mr_base(repo, number)))
        .await?;
    let (required_approvals, approvals) = if approvals_response.ok() {
        let value = approvals_response
            .json()
            .map_err(|_| GithubError::Unreadable)?;
        (
            value
                .get("approvals_required")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            value
                .get("approved_by")
                .and_then(Value::as_array)
                .map_or(0, |a| a.len() as u64),
        )
    } else if approvals_response.status == 404 {
        (0, 0)
    } else {
        return Err(interpret_failure(&approvals_response));
    };
    let pipeline_required = flag(&project, "only_allow_merge_if_pipeline_succeeds")
        || matches!(status.as_str(), "ci_must_pass" | "ci_still_running");
    let required_checks = if pipeline_required {
        vec![RequiredCheck {
            name: "Pipeline".to_owned(),
            state: mr
                .get("head_pipeline")
                .filter(|p| !p.is_null())
                .and_then(|p| text(p, "status"))
                .map(|s| pipeline_state(&s)),
        }]
    } else {
        Vec::new()
    };
    let mut blockers = Vec::new();
    if state != PullState::Open {
        blockers.push("la merge request n'est plus ouverte".to_owned());
    }
    if draft {
        blockers.push("la merge request est un brouillon".to_owned());
    }
    blockers.extend(explain(&status));
    for check in &required_checks {
        match check.state {
            Some(CheckState::Success | CheckState::Skipped) => {}
            Some(CheckState::Pending) => blockers.push(format!(
                "la vérification requise « {} » est en attente",
                check.name
            )),
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
        blockers.push(format!(
            "{approvals} approbation(s) sur {required_approvals} requise(s)"
        ));
    } else if status == "not_approved" {
        blockers.push("des règles d'approbation du projet ne sont pas satisfaites".to_owned());
    }
    if protected_refusal {
        blockers.push(format!(
            "la branche cible « {target} » est protégée : votre niveau d'accès ne permet pas d'y fusionner"
        ));
    }
    Ok(MergeReadiness {
        head_sha,
        state,
        draft,
        mergeable_state: status,
        methods,
        required_checks,
        required_approvals,
        approvals,
        protection_unreadable,
        blockers,
    })
}

pub(crate) async fn is_merged(
    client: &ApiClient,
    repo: Repo<'_>,
    number: u64,
) -> Result<bool, GithubError> {
    checked(repo)?;
    let value = get_json(client, &mr_base(repo, number)).await?;
    Ok(text(&value, "state").as_deref() == Some("merged"))
}

pub(crate) async fn merge_pull(
    client: &ApiClient,
    _connection: &Connection,
    repo: Repo<'_>,
    number: u64,
    request: &MergeRequest,
) -> Result<HostedMerge, GithubError> {
    let head = current_head(client, repo, number).await?;
    if head != request.expected_head {
        return Err(GithubError::AnchorStale {
            prepared: request.expected_head.chars().take(7).collect(),
            current: head.chars().take(7).collect(),
        });
    }
    let mut body = json!({
        "sha": request.expected_head,
        "squash": request.method == MergeMethod::Squash,
        "should_remove_source_branch": false,
    });
    let message = request.message.trim();
    if !message.is_empty() {
        match request.method {
            MergeMethod::Squash => body["squash_commit_message"] = json!(message),
            MergeMethod::Merge => body["merge_commit_message"] = json!(message),
            MergeMethod::Rebase => {}
        }
    }
    let response = client
        .send(
            "PUT",
            &format!("{}/merge", mr_base(repo, number)),
            Some(body),
        )
        .await?;
    match response.status {
        401 => {
            return Err(GithubError::MergeRefused(
                "vous n'avez pas le droit de fusionner cette merge request (branche protégée ou rôle insuffisant)".to_owned(),
            ));
        }
        406 => {
            return Err(GithubError::MergeRefused(
                "la merge request ne peut pas être fusionnée dans son état actuel".to_owned(),
            ));
        }
        _ if !response.ok() => return Err(failure(&response)),
        _ => {}
    }
    let value = response.json().map_err(|_| GithubError::Unreadable)?;
    if text(&value, "state").as_deref() != Some("merged") {
        return Err(GithubError::MergeRefused(format!(
            "la fusion n'a pas été effectuée (état {})",
            text(&value, "state").unwrap_or_default()
        )));
    }
    let sha = text(&value, "merge_commit_sha")
        .or_else(|| text(&value, "squash_commit_sha"))
        .unwrap_or_default();
    let verified_merged = is_merged(client, repo, number).await?;
    Ok(HostedMerge {
        sha,
        verified_merged,
    })
}
