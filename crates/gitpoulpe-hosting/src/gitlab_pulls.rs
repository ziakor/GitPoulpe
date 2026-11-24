use gitpoulpe_core::Connection;
use serde_json::{Value, json};

use crate::attachments::{Change, gitlab_attachments, gitlab_file_media, gitlab_web};
use crate::github::{GithubError, interpret_failure};
use crate::gitlab_repos::{parse_project, project_id, valid_namespace};
use crate::http::ApiClient;
use crate::pull_read::{PullCommit, PullDetail, PullFile, PullFilter, PullState, PullSummary};
use crate::pulls::{
    ChangedFile, Comparison, HostedPull, MAX_FILES, PullDraft, PullTarget, encode, valid_branch,
};
use crate::repos::{RepoSummary, pages, valid_org};

const DRAFT_PREFIX: &str = "Draft: ";

struct Project {
    id: u64,
    repo: RepoSummary,
}

pub(crate) fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

pub(crate) fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub(crate) fn valid_path(owner: &str, name: &str) -> bool {
    valid_namespace(owner) && valid_org(name)
}

pub(crate) fn project_path(owner: &str, name: &str) -> String {
    encode(&format!("{owner}/{name}"))
}

async fn get_project(
    client: &ApiClient,
    connection: &Connection,
    owner: &str,
    name: &str,
) -> Result<Project, GithubError> {
    if !valid_path(owner, name) {
        return Err(GithubError::NotFound);
    }
    let response = client
        .get(&format!("projects/{}", project_path(owner, name)))
        .await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    let value = response.json().map_err(|_| GithubError::Unreadable)?;
    Ok(Project {
        id: project_id(&value).ok_or(GithubError::Unreadable)?,
        repo: parse_project(connection, &value).ok_or(GithubError::Unreadable)?,
    })
}

fn parse_hosted(value: &Value) -> Option<HostedPull> {
    let draft = flag(value, "draft");
    let title = text(value, "title").unwrap_or_default();
    Some(HostedPull {
        number: value.get("iid")?.as_u64()?,
        url: text(value, "web_url")?,
        title: strip_draft(&title, draft),
        draft,
        state: match text(value, "state").as_deref() {
            Some("opened" | "locked") => "open".to_owned(),
            Some(other) => other.to_owned(),
            None => String::new(),
        },
        head: text(value, "source_branch").unwrap_or_default(),
        base: text(value, "target_branch").unwrap_or_default(),
    })
}

fn strip_draft(title: &str, draft: bool) -> String {
    match title.strip_prefix(DRAFT_PREFIX) {
        Some(rest) if draft => rest.to_owned(),
        _ => title.to_owned(),
    }
}

fn parse_summary(value: &Value) -> Option<PullSummary> {
    let draft = flag(value, "draft");
    let refs = value.get("diff_refs").filter(|r| !r.is_null());
    Some(PullSummary {
        number: value.get("iid")?.as_u64()?,
        title: strip_draft(&text(value, "title").unwrap_or_default(), draft),
        state: match text(value, "state")?.as_str() {
            "opened" | "locked" => PullState::Open,
            "merged" => PullState::Merged,
            _ => PullState::Closed,
        },
        draft,
        author: value
            .get("author")
            .and_then(|a| text(a, "username"))
            .unwrap_or_default(),
        head_ref: text(value, "source_branch")?,
        head_sha: text(value, "sha").unwrap_or_default(),
        base_ref: text(value, "target_branch")?,
        base_sha: refs.and_then(|r| text(r, "base_sha")).unwrap_or_default(),
        url: text(value, "web_url").unwrap_or_default(),
        updated_at: text(value, "updated_at").unwrap_or_default(),
    })
}

fn count_lines(diff: &str) -> (u64, u64) {
    diff.lines().fold((0, 0), |(added, removed), line| {
        match line.as_bytes().first() {
            Some(b'+') => (added + 1, removed),
            Some(b'-') => (added, removed + 1),
            _ => (added, removed),
        }
    })
}

fn parse_change(value: &Value) -> Option<PullFile> {
    let path = text(value, "new_path").or_else(|| text(value, "old_path"))?;
    let status = if flag(value, "new_file") {
        "added"
    } else if flag(value, "deleted_file") {
        "removed"
    } else if flag(value, "renamed_file") {
        "renamed"
    } else {
        "modified"
    };
    let diff = text(value, "diff").unwrap_or_default();
    let (additions, deletions) = count_lines(&diff);
    let binary = diff.is_empty() || diff.starts_with("Binary files ");
    Some(PullFile {
        change: ChangedFile {
            path,
            status: status.to_owned(),
            additions,
            deletions,
        },
        patch: (!binary).then_some(diff),
        media: None,
    })
}

async fn branch_published(
    client: &ApiClient,
    project: &str,
    branch: &str,
) -> Result<bool, GithubError> {
    let response = client
        .get(&format!(
            "projects/{project}/repository/branches/{}",
            encode(branch)
        ))
        .await?;
    match response.status {
        404 => Ok(false),
        _ if response.ok() => Ok(true),
        _ => Err(interpret_failure(&response)),
    }
}

async fn open_mr_for(
    client: &ApiClient,
    target: &str,
    head_id: u64,
    branch: &str,
    base: &str,
) -> Result<Option<HostedPull>, GithubError> {
    let raw = pages(
        client,
        format!(
            "projects/{target}/merge_requests?state=opened&source_branch={}&target_branch={}&per_page=100",
            encode(branch),
            encode(base)
        ),
    )
    .await?;
    Ok(raw
        .items
        .iter()
        .filter(|mr| mr.get("source_project_id").and_then(Value::as_u64) == Some(head_id))
        .find_map(parse_hosted))
}

async fn compare(
    client: &ApiClient,
    into: &str,
    from: &str,
    to: &str,
    from_project: Option<u64>,
) -> Result<Value, GithubError> {
    let mut url = format!(
        "projects/{into}/repository/compare?from={}&to={}",
        encode(from),
        encode(to)
    );
    if let Some(id) = from_project {
        url.push_str(&format!("&from_project_id={id}"));
    }
    let response = client.get(&url).await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    response.json().map_err(|_| GithubError::Unreadable)
}

pub(crate) async fn prepare_mr(
    client: &ApiClient,
    connection: &Connection,
    head: (&str, &str),
    branch: &str,
    to_parent: bool,
    base: Option<&str>,
) -> Result<PullTarget, GithubError> {
    if !valid_branch(branch) || base.is_some_and(|b| !valid_branch(b)) {
        return Err(GithubError::NotFound);
    }
    let head_project = get_project(client, connection, head.0, head.1).await?;
    let target_project = match head_project.repo.parent.as_deref().filter(|_| to_parent) {
        Some(parent) => {
            let (owner, name) = parent.rsplit_once('/').ok_or(GithubError::Unreadable)?;
            get_project(client, connection, owner, name).await?
        }
        None => Project {
            id: head_project.id,
            repo: head_project.repo.clone(),
        },
    };
    let base = match base {
        Some(base) => base.to_owned(),
        None => target_project
            .repo
            .default_branch
            .clone()
            .ok_or(GithubError::Unreadable)?,
    };
    let head_path = project_path(&head_project.repo.owner, &head_project.repo.name);
    let target_path = project_path(&target_project.repo.owner, &target_project.repo.name);
    let cross = head_project.id != target_project.id;
    let published = branch_published(client, &head_path, branch).await?;
    let (comparison, existing) = if published {
        let ahead = compare(
            client,
            &head_path,
            &base,
            branch,
            cross.then_some(target_project.id),
        )
        .await?;
        let behind = compare(
            client,
            &target_path,
            branch,
            &base,
            cross.then_some(head_project.id),
        )
        .await?;
        let files: Vec<ChangedFile> = ahead
            .get("diffs")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(parse_change)
                    .map(|f| f.change)
                    .collect()
            })
            .unwrap_or_default();
        let count = |value: &Value| {
            value
                .get("commits")
                .and_then(Value::as_array)
                .map_or(0, |c| c.len() as u64)
        };
        let comparison = Comparison {
            ahead: count(&ahead),
            behind: count(&behind),
            files_truncated: files.len() >= MAX_FILES || flag(&ahead, "compare_timeout"),
            files,
        };
        let existing = open_mr_for(client, &target_path, head_project.id, branch, &base).await?;
        (Some(comparison), existing)
    } else {
        (None, None)
    };
    Ok(PullTarget {
        head_repo: head_project.repo,
        target_repo: target_project.repo,
        branch: branch.to_owned(),
        base,
        published,
        comparison,
        existing,
    })
}

pub(crate) async fn create_mr(
    client: &ApiClient,
    connection: &Connection,
    prepared: &PullTarget,
    draft: &PullDraft,
) -> Result<HostedPull, GithubError> {
    let title = draft.title.trim();
    if title.is_empty() {
        return Err(GithubError::Validation(
            "le titre est obligatoire".to_owned(),
        ));
    }
    if !prepared.published {
        return Err(GithubError::BranchNotPublished(prepared.branch.clone()));
    }
    let head_project = get_project(
        client,
        connection,
        &prepared.head_repo.owner,
        &prepared.head_repo.name,
    )
    .await?;
    let target_project = get_project(
        client,
        connection,
        &prepared.target_repo.owner,
        &prepared.target_repo.name,
    )
    .await?;
    let target_path = project_path(&prepared.target_repo.owner, &prepared.target_repo.name);
    if let Some(existing) = open_mr_for(
        client,
        &target_path,
        head_project.id,
        &prepared.branch,
        &prepared.base,
    )
    .await?
    {
        return Err(GithubError::PullExists(existing.number, existing.url));
    }
    let full_title = if draft.draft && !title.starts_with(DRAFT_PREFIX) {
        format!("{DRAFT_PREFIX}{title}")
    } else {
        title.to_owned()
    };
    let mut body = json!({
        "source_branch": prepared.branch,
        "target_branch": prepared.base,
        "title": full_title,
        "description": draft.body,
    });
    if head_project.id != target_project.id {
        body["target_project_id"] = json!(target_project.id);
    }
    let response = client
        .send(
            "POST",
            &format!(
                "projects/{}/merge_requests",
                project_path(&prepared.head_repo.owner, &prepared.head_repo.name)
            ),
            Some(body),
        )
        .await?;
    if response.status == 409
        && let Some(existing) = open_mr_for(
            client,
            &target_path,
            head_project.id,
            &prepared.branch,
            &prepared.base,
        )
        .await?
    {
        return Err(GithubError::PullExists(existing.number, existing.url));
    }
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    let value = response.json().map_err(|_| GithubError::Unreadable)?;
    parse_hosted(&value).ok_or(GithubError::Unreadable)
}

pub(crate) async fn list_mrs(
    client: &ApiClient,
    repo: (&str, &str),
    filter: PullFilter,
) -> Result<(Vec<PullSummary>, bool), GithubError> {
    if !valid_path(repo.0, repo.1) {
        return Err(GithubError::NotFound);
    }
    let state = match filter {
        PullFilter::Open => "opened",
        PullFilter::Closed | PullFilter::All => "all",
    };
    let raw = pages(
        client,
        format!(
            "projects/{}/merge_requests?state={state}&order_by=updated_at&sort=desc&per_page=100",
            project_path(repo.0, repo.1)
        ),
    )
    .await?;
    let items = raw
        .items
        .iter()
        .filter_map(parse_summary)
        .filter(|s| filter != PullFilter::Closed || s.state != PullState::Open)
        .collect();
    Ok((items, raw.truncated))
}

pub(crate) async fn get_json(client: &ApiClient, url: &str) -> Result<Value, GithubError> {
    let response = client.get(url).await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    response.json().map_err(|_| GithubError::Unreadable)
}

pub(crate) async fn load_mr(
    client: &ApiClient,
    repo: (&str, &str),
    number: u64,
) -> Result<PullDetail, GithubError> {
    if !valid_path(repo.0, repo.1) {
        return Err(GithubError::NotFound);
    }
    let base = format!(
        "projects/{}/merge_requests/{number}",
        project_path(repo.0, repo.1)
    );
    let value = get_json(client, &base).await?;
    let summary = parse_summary(&value).ok_or(GithubError::Unreadable)?;
    let commits_raw = pages(client, format!("{base}/commits?per_page=100")).await?;
    let commits: Vec<PullCommit> = commits_raw
        .items
        .iter()
        .filter_map(|c| {
            Some(PullCommit {
                sha: text(c, "id")?,
                subject: text(c, "title").unwrap_or_default(),
                author: text(c, "author_name").unwrap_or_default(),
            })
        })
        .collect();
    let (changes, files_truncated) = fetch_changes(client, &base).await?;
    let web = gitlab_web(client.base());
    let files: Vec<PullFile> = changes
        .iter()
        .filter_map(|value| {
            let mut file = parse_change(value)?;
            if file.patch.is_none() {
                let previous = text(value, "old_path");
                file.media = Some(gitlab_file_media(
                    web,
                    repo,
                    Change {
                        path: &file.change.path,
                        previous: previous.as_deref(),
                        status: &file.change.status,
                    },
                    Some(&summary.base_sha),
                    &summary.head_sha,
                ));
            }
            Some(file)
        })
        .collect();
    let after = get_json(client, &base).await?;
    let changed_during_load = text(&after, "sha") != text(&value, "sha");
    let body = text(&value, "description").unwrap_or_default();
    Ok(PullDetail {
        attachments: gitlab_attachments(&body, web, repo),
        body,
        files_truncated,
        summary,
        commits,
        files,
        changed_during_load,
    })
}

pub(crate) fn mr_base(repo: (&str, &str), number: u64) -> String {
    format!(
        "projects/{}/merge_requests/{number}",
        project_path(repo.0, repo.1)
    )
}

pub(crate) async fn fetch_changes(
    client: &ApiClient,
    base: &str,
) -> Result<(Vec<Value>, bool), GithubError> {
    match pages(client, format!("{base}/diffs?per_page=100")).await {
        Ok(raw) => Ok((raw.items, raw.truncated)),
        Err(GithubError::NotFound) => {
            let legacy = get_json(client, &format!("{base}/changes")).await?;
            Ok((
                legacy
                    .get("changes")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
                flag(&legacy, "overflow"),
            ))
        }
        Err(other) => Err(other),
    }
}
