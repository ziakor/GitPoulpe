use gitpoulpe_core::Connection;
use serde_json::{Value, json};

use crate::attachments::{gitlab_attachments, gitlab_web};
use crate::github::{GithubError, interpret_failure};
use crate::gitlab_pulls::{fetch_changes, flag, get_json, mr_base, text, valid_path};
use crate::http::{ApiClient, Response};
use crate::pull_comments::{IssueComment, LineAnchor, PullComments, ReviewComment, Side};
use crate::pull_review::{DraftLine, Review, ReviewDecision, ReviewSubmission};
use crate::repos::pages;

type Repo<'a> = (&'a str, &'a str);

pub(crate) fn failure(response: &Response) -> GithubError {
    if response.status != 400 {
        return interpret_failure(response);
    }
    let detail = response.json().ok().and_then(|v| match v.get("message") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
        None => None,
    });
    GithubError::Validation(detail.unwrap_or_else(|| "requête refusée".to_owned()))
}

fn author(value: &Value) -> String {
    value
        .get("author")
        .and_then(|a| text(a, "username"))
        .unwrap_or_default()
}

fn checked(repo: Repo<'_>) -> Result<(), GithubError> {
    if valid_path(repo.0, repo.1) {
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

fn attachments_of(note: &Value, client: &ApiClient, repo: Repo<'_>) -> Vec<crate::Attachment> {
    gitlab_attachments(
        note.get("body").and_then(Value::as_str).unwrap_or(""),
        gitlab_web(client.base()),
        repo,
    )
}

fn parse_issue(note: &Value, client: &ApiClient, repo: Repo<'_>) -> Option<IssueComment> {
    Some(IssueComment {
        attachments: attachments_of(note, client, repo),
        id: note.get("id")?.as_u64()?,
        author: author(note),
        body: text(note, "body").unwrap_or_default(),
        created_at: text(note, "created_at").unwrap_or_default(),
    })
}

fn parse_diff_note(
    note: &Value,
    root: Option<&Value>,
    reply_to: Option<u64>,
    head: &str,
    place: (&ApiClient, Repo<'_>),
) -> Option<ReviewComment> {
    let position = note
        .get("position")
        .filter(|p| !p.is_null())
        .or_else(|| root?.get("position").filter(|p| !p.is_null()))?;
    let new_line = position.get("new_line").and_then(Value::as_u64);
    let old_line = position.get("old_line").and_then(Value::as_u64);
    let commit_sha = text(position, "head_sha").unwrap_or_default();
    Some(ReviewComment {
        attachments: attachments_of(note, place.0, place.1),
        id: note.get("id")?.as_u64()?,
        author: author(note),
        body: text(note, "body").unwrap_or_default(),
        path: text(position, "new_path").or_else(|| text(position, "old_path"))?,
        line: new_line.or(old_line),
        side: if new_line.is_some() {
            Side::Right
        } else {
            Side::Left
        },
        outdated: commit_sha != head,
        commit_sha,
        in_reply_to: reply_to,
        created_at: text(note, "created_at").unwrap_or_default(),
    })
}

struct Refs {
    head: String,
    base: String,
    start: String,
}

async fn refs(client: &ApiClient, repo: Repo<'_>, number: u64) -> Result<Refs, GithubError> {
    let value = get_json(client, &mr_base(repo, number)).await?;
    let diff = value.get("diff_refs").filter(|r| !r.is_null());
    Ok(Refs {
        head: text(&value, "sha").ok_or(GithubError::Unreadable)?,
        base: diff
            .and_then(|r| text(r, "base_sha"))
            .ok_or(GithubError::Unreadable)?,
        start: diff
            .and_then(|r| text(r, "start_sha"))
            .ok_or(GithubError::Unreadable)?,
    })
}

pub(crate) async fn current_head(
    client: &ApiClient,
    repo: Repo<'_>,
    number: u64,
) -> Result<String, GithubError> {
    checked(repo)?;
    Ok(refs(client, repo, number).await?.head)
}

async fn discussions(
    client: &ApiClient,
    repo: Repo<'_>,
    number: u64,
) -> Result<Vec<Value>, GithubError> {
    Ok(pages(
        client,
        format!("{}/discussions?per_page=100", mr_base(repo, number)),
    )
    .await?
    .items)
}

fn notes_of(discussion: &Value) -> Vec<&Value> {
    discussion
        .get("notes")
        .and_then(Value::as_array)
        .map(|n| n.iter().filter(|n| !flag(n, "system")).collect())
        .unwrap_or_default()
}

pub(crate) async fn list_comments(
    client: &ApiClient,
    repo: Repo<'_>,
    number: u64,
) -> Result<PullComments, GithubError> {
    checked(repo)?;
    let head = current_head(client, repo, number).await?;
    let mut comments = PullComments::default();
    for discussion in discussions(client, repo, number).await? {
        let notes = notes_of(&discussion);
        let root = notes.first().copied();
        let root_id = root.and_then(|n| n.get("id").and_then(Value::as_u64));
        for (index, note) in notes.iter().enumerate() {
            let reply_to = if index == 0 { None } else { root_id };
            match parse_diff_note(note, root, reply_to, &head, (client, repo)) {
                Some(found) => comments.review.push(found),
                None => comments.issue.extend(parse_issue(note, client, repo)),
            }
        }
    }
    Ok(comments)
}

pub(crate) async fn post_comment(
    client: &ApiClient,
    repo: Repo<'_>,
    number: u64,
    body: &str,
) -> Result<IssueComment, GithubError> {
    checked(repo)?;
    let body = nonempty(body)?;
    let response = client
        .send(
            "POST",
            &format!("{}/notes", mr_base(repo, number)),
            Some(json!({ "body": body })),
        )
        .await?;
    if !response.ok() {
        return Err(failure(&response));
    }
    parse_issue(
        &response.json().map_err(|_| GithubError::Unreadable)?,
        client,
        repo,
    )
    .ok_or(GithubError::Unreadable)
}

fn find_line(diff: &str, side: Side, wanted: u64) -> Option<(Option<u64>, Option<u64>)> {
    let (mut old, mut new) = (0u64, 0u64);
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("@@ -") {
            let (old_part, rest) = rest.split_once(' ')?;
            let new_part = rest.strip_prefix('+')?.split(' ').next()?;
            old = old_part.split(',').next()?.parse().ok()?;
            new = new_part.split(',').next()?.parse().ok()?;
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => {
                if side == Side::Right && new == wanted {
                    return Some((None, Some(new)));
                }
                new += 1;
            }
            Some(b'-') => {
                if side == Side::Left && old == wanted {
                    return Some((Some(old), None));
                }
                old += 1;
            }
            Some(b' ') => {
                let hit = match side {
                    Side::Right => new == wanted,
                    Side::Left => old == wanted,
                };
                if hit {
                    return Some((Some(old), Some(new)));
                }
                old += 1;
                new += 1;
            }
            _ => {}
        }
    }
    None
}

struct Located {
    old_path: String,
    old_line: Option<u64>,
    new_line: Option<u64>,
}

fn locate(
    changes: &[Value],
    truncated: bool,
    path: &str,
    side: Side,
    line: u64,
) -> Result<Located, GithubError> {
    let entry = changes
        .iter()
        .find(|c| text(c, "new_path").as_deref() == Some(path))
        .ok_or_else(|| {
            GithubError::Validation(if truncated {
                format!(
                    "{path} n'est pas dans la partie chargée des diffs : GitLab a tronqué la liste"
                )
            } else {
                format!("{path} ne fait pas partie des changements de la merge request")
            })
        })?;
    let diff = text(entry, "diff").unwrap_or_default();
    let (old_line, new_line) = find_line(&diff, side, line).ok_or_else(|| {
        GithubError::Validation(format!(
            "la ligne {line} de {path} n'est pas dans le diff : GitLab ne permet de commenter que les lignes affichées"
        ))
    })?;
    Ok(Located {
        old_path: text(entry, "old_path").unwrap_or_else(|| path.to_owned()),
        old_line,
        new_line,
    })
}

fn position(refs: &Refs, path: &str, found: &Located) -> Value {
    let mut position = json!({
        "position_type": "text",
        "base_sha": refs.base,
        "start_sha": refs.start,
        "head_sha": refs.head,
        "new_path": path,
        "old_path": found.old_path,
    });
    if let Some(line) = found.new_line {
        position["new_line"] = json!(line);
    }
    if let Some(line) = found.old_line {
        position["old_line"] = json!(line);
    }
    position
}

fn single_line(start: Option<u64>, line: u64) -> Result<(), GithubError> {
    if line == 0 || start.is_some_and(|s| s == 0 || s > line) {
        return Err(GithubError::Validation(
            "plage de lignes invalide".to_owned(),
        ));
    }
    if start.is_some_and(|s| s != line) {
        return Err(GithubError::Validation(
            "GitLab ne permet pas de commenter une plage de lignes : commentez une ligne à la fois"
                .to_owned(),
        ));
    }
    Ok(())
}

fn stale(prepared: &str, current: &str) -> GithubError {
    GithubError::AnchorStale {
        prepared: prepared.chars().take(7).collect(),
        current: current.chars().take(7).collect(),
    }
}

pub(crate) async fn post_line_comment(
    client: &ApiClient,
    repo: Repo<'_>,
    number: u64,
    anchor: &LineAnchor,
    body: &str,
) -> Result<ReviewComment, GithubError> {
    checked(repo)?;
    let body = nonempty(body)?;
    single_line(anchor.start_line, anchor.line)?;
    let current = refs(client, repo, number).await?;
    if current.head != anchor.commit_sha {
        return Err(stale(&anchor.commit_sha, &current.head));
    }
    let (changes, truncated) = fetch_changes(client, &mr_base(repo, number)).await?;
    let found = locate(&changes, truncated, &anchor.path, anchor.side, anchor.line)?;
    let response = client
        .send(
            "POST",
            &format!("{}/discussions", mr_base(repo, number)),
            Some(json!({ "body": body, "position": position(&current, &anchor.path, &found) })),
        )
        .await?;
    if !response.ok() {
        return Err(failure(&response));
    }
    let discussion = response.json().map_err(|_| GithubError::Unreadable)?;
    notes_of(&discussion)
        .first()
        .and_then(|n| parse_diff_note(n, None, None, &current.head, (client, repo)))
        .ok_or(GithubError::Unreadable)
}

pub(crate) async fn reply_to_comment(
    client: &ApiClient,
    repo: Repo<'_>,
    number: u64,
    comment_id: u64,
    body: &str,
) -> Result<ReviewComment, GithubError> {
    checked(repo)?;
    let body = nonempty(body)?;
    let head = current_head(client, repo, number).await?;
    let all = discussions(client, repo, number).await?;
    let discussion = all
        .iter()
        .find(|d| {
            notes_of(d)
                .iter()
                .any(|n| n.get("id").and_then(Value::as_u64) == Some(comment_id))
        })
        .ok_or(GithubError::NotFound)?;
    let id = text(discussion, "id").ok_or(GithubError::Unreadable)?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(GithubError::Unreadable);
    }
    let root = notes_of(discussion).first().copied().cloned();
    let root_id = root
        .as_ref()
        .and_then(|n| n.get("id").and_then(Value::as_u64));
    let response = client
        .send(
            "POST",
            &format!("{}/discussions/{id}/notes", mr_base(repo, number)),
            Some(json!({ "body": body })),
        )
        .await?;
    if !response.ok() {
        return Err(failure(&response));
    }
    let note = response.json().map_err(|_| GithubError::Unreadable)?;
    parse_diff_note(&note, root.as_ref(), root_id, &head, (client, repo))
        .ok_or(GithubError::Unreadable)
}

pub(crate) async fn list_reviews(
    client: &ApiClient,
    repo: Repo<'_>,
    number: u64,
) -> Result<Vec<Review>, GithubError> {
    checked(repo)?;
    let head = current_head(client, repo, number).await?;
    let mut notes = Vec::new();
    for discussion in discussions(client, repo, number).await? {
        if !flag(&discussion, "individual_note") {
            continue;
        }
        for note in notes_of(&discussion) {
            notes.push(Review {
                id: note.get("id").and_then(Value::as_u64).unwrap_or(0),
                author: author(note),
                state: "COMMENTED".to_owned(),
                body: text(note, "body").unwrap_or_default(),
                commit_sha: head.clone(),
                submitted_at: text(note, "created_at").unwrap_or_default(),
            });
        }
    }
    let mut reviews = Vec::new();
    let approvals = client
        .get(&format!("{}/approvals", mr_base(repo, number)))
        .await?;
    if approvals.ok() {
        let value = approvals.json().map_err(|_| GithubError::Unreadable)?;
        for entry in value
            .get("approved_by")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let user = entry.get("user").unwrap_or(entry);
            let login = text(user, "username").unwrap_or_default();
            let summary = notes
                .iter()
                .rev()
                .find(|n| n.author.eq_ignore_ascii_case(&login))
                .map(|n| n.body.clone())
                .unwrap_or_default();
            reviews.push(Review {
                id: user.get("id").and_then(Value::as_u64).unwrap_or(0),
                author: login,
                state: "APPROVED".to_owned(),
                body: summary,
                commit_sha: head.clone(),
                submitted_at: String::new(),
            });
        }
    } else if approvals.status != 404 {
        return Err(interpret_failure(&approvals));
    }
    reviews.extend(notes);
    Ok(reviews)
}

async fn drop_drafts(client: &ApiClient, base: &str, ids: &[u64]) {
    for id in ids {
        let _ = client
            .send("DELETE", &format!("{base}/draft_notes/{id}"), None)
            .await;
    }
}

async fn create_drafts(
    client: &ApiClient,
    base: &str,
    drafts: &[Value],
    created: &mut Vec<u64>,
) -> Result<(), GithubError> {
    for draft in drafts {
        let response = client
            .send("POST", &format!("{base}/draft_notes"), Some(draft.clone()))
            .await?;
        if !response.ok() {
            return Err(failure(&response));
        }
        let value = response.json().map_err(|_| GithubError::Unreadable)?;
        created.push(
            value
                .get("id")
                .and_then(Value::as_u64)
                .ok_or(GithubError::Unreadable)?,
        );
    }
    Ok(())
}

fn validate(submission: &ReviewSubmission) -> Result<(), GithubError> {
    let body = submission.body.trim();
    if submission.decision == ReviewDecision::RequestChanges {
        return Err(GithubError::Validation(
            "GitLab ne permet pas de demander des changements par l'API : publiez vos commentaires, puis utilisez GitLab".to_owned(),
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
    if body.is_empty() && !submission.lines.is_empty() {
        return Err(GithubError::Validation(
            "sur GitLab, ajoutez un message à la revue : il permet de vérifier que l'envoi a bien eu lieu".to_owned(),
        ));
    }
    for line in &submission.lines {
        if line.body.trim().is_empty() {
            return Err(GithubError::Validation(format!(
                "commentaire invalide sur {}",
                line.path
            )));
        }
        single_line(line.start_line, line.line)?;
    }
    Ok(())
}

pub(crate) async fn submit_review(
    client: &ApiClient,
    connection: &Connection,
    repo: Repo<'_>,
    number: u64,
    submission: &ReviewSubmission,
) -> Result<Review, GithubError> {
    checked(repo)?;
    validate(submission)?;
    let current = refs(client, repo, number).await?;
    if current.head != submission.commit_sha {
        return Err(stale(&submission.commit_sha, &current.head));
    }
    let base = mr_base(repo, number);
    let mut drafts = Vec::new();
    if !submission.lines.is_empty() {
        let (changes, truncated) = fetch_changes(client, &base).await?;
        for DraftLine {
            path,
            line,
            side,
            body,
            ..
        } in &submission.lines
        {
            let found = locate(&changes, truncated, path, *side, *line)?;
            drafts
                .push(json!({ "note": body.trim(), "position": position(&current, path, &found) }));
        }
    }
    let summary = submission.body.trim();
    if !summary.is_empty() {
        drafts.push(json!({ "note": summary }));
    }
    let mut created = Vec::new();
    if let Err(error) = create_drafts(client, &base, &drafts, &mut created).await {
        drop_drafts(client, &base, &created).await;
        return Err(error);
    }
    if submission.decision == ReviewDecision::Approve {
        let outcome = client
            .send(
                "POST",
                &format!("{base}/approve"),
                Some(json!({ "sha": current.head })),
            )
            .await;
        let refusal = match outcome {
            Ok(response) if response.ok() => None,
            Ok(response) if response.status == 401 => Some(GithubError::Validation(
                "GitLab refuse l'approbation : vous ne pouvez pas approuver cette merge request (auteur, règle d'approbation ou révision modifiée)".to_owned(),
            )),
            Ok(response) => Some(failure(&response)),
            Err(error) => Some(error.into()),
        };
        if let Some(error) = refusal {
            drop_drafts(client, &base, &created).await;
            return Err(error);
        }
    }
    if !created.is_empty() {
        let response = client
            .send("POST", &format!("{base}/draft_notes/bulk_publish"), None)
            .await?;
        if !response.ok() {
            return Err(failure(&response));
        }
    }
    Ok(Review {
        id: created.first().copied().unwrap_or(0),
        author: connection.login.clone(),
        state: if submission.decision == ReviewDecision::Approve {
            "APPROVED"
        } else {
            "COMMENTED"
        }
        .to_owned(),
        body: summary.to_owned(),
        commit_sha: current.head,
        submitted_at: String::new(),
    })
}
