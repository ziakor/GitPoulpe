mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{
    CheckState, DraftLine, GithubError, LineAnchor, MemoryStore, ReviewDecision, ReviewSubmission,
    SecretStore, Side, current_head, list_comments, list_reviews, load_checks, post_comment,
    post_line_comment, reply_to_comment, review_exists, submit_review,
};

const API: (&str, &str) = ("acme/platform", "api");
const HEAD: &str = "2222222222222222222222222222222222222222";

fn connection(api: &FakeApi) -> Connection {
    Connection {
        id: "gl/octo".to_owned(),
        provider: Provider::Gitlab,
        api: format!("{}/api/v4", api.base()),
        web: api.base(),
        login: "Octo".to_owned(),
        scopes: Vec::new(),
    }
}

fn stored(connection: &Connection) -> MemoryStore {
    let store = MemoryStore::default();
    store.set(&connection.secret_key(), "glgood").unwrap();
    store
}

fn writes(api: &FakeApi) -> Vec<(String, String, serde_json::Value)> {
    api.requests()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["method"].is_string())
        .map(|v| {
            (
                v["method"].as_str().unwrap().to_owned(),
                v["path"].as_str().unwrap().to_owned(),
                serde_json::from_str(v["body"].as_str().unwrap()).unwrap_or_default(),
            )
        })
        .collect()
}

fn anchor(path: &str, line: u64, side: Side, sha: &str) -> LineAnchor {
    LineAnchor {
        commit_sha: sha.to_owned(),
        path: path.to_owned(),
        line,
        start_line: None,
        side,
    }
}

fn line(path: &str, line: u64, side: Side, body: &str) -> DraftLine {
    DraftLine {
        path: path.to_owned(),
        line,
        start_line: None,
        side,
        body: body.to_owned(),
    }
}

fn submission(
    decision: ReviewDecision,
    body: &str,
    lines: Vec<DraftLine>,
    sha: &str,
) -> ReviewSubmission {
    ReviewSubmission {
        commit_sha: sha.to_owned(),
        body: body.to_owned(),
        decision,
        lines,
    }
}

#[tokio::test]
async fn discussions_become_general_and_line_comments_with_replies_and_outdated_marks() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let comments = list_comments(&store, &conn, API.0, API.1, 20)
        .await
        .unwrap();
    assert_eq!(comments.issue.len(), 1);
    assert_eq!(
        (comments.issue[0].id, comments.issue[0].body.as_str()),
        (501, "General remark")
    );
    let ids: Vec<u64> = comments.review.iter().map(|c| c.id).collect();
    assert_eq!(ids, [601, 602, 603]);
    let root = &comments.review[0];
    assert_eq!(
        (root.path.as_str(), root.line, root.side),
        ("src/a.rs", Some(3), Side::Right)
    );
    assert!(!root.outdated && root.in_reply_to.is_none());
    assert_eq!(comments.review[1].in_reply_to, Some(601));
    let old = &comments.review[2];
    assert_eq!(
        (old.line, old.side, old.outdated),
        (Some(2), Side::Left, true)
    );
}

#[tokio::test]
async fn a_general_comment_is_posted_and_an_empty_one_never_leaves() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let posted = post_comment(&store, &conn, API.0, API.1, 20, "  Looks fine  ")
        .await
        .unwrap();
    assert_eq!(
        (posted.author.as_str(), posted.body.as_str()),
        ("Octo", "Looks fine")
    );
    let empty = post_comment(&store, &conn, API.0, API.1, 20, "  ").await;
    assert!(matches!(empty, Err(GithubError::Validation(_))));
    assert_eq!(writes(&api).len(), 1);
}

#[tokio::test]
async fn line_comments_carry_the_exact_revision_and_the_right_old_and_new_line_numbers() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let added = post_line_comment(
        &store,
        &conn,
        API.0,
        API.1,
        20,
        &anchor("src/a.rs", 3, Side::Right, HEAD),
        "Why?",
    )
    .await
    .unwrap();
    assert_eq!(
        (added.path.as_str(), added.line, added.side),
        ("src/a.rs", Some(3), Side::Right)
    );
    let context = post_line_comment(
        &store,
        &conn,
        API.0,
        API.1,
        20,
        &anchor("src/a.rs", 1, Side::Right, HEAD),
        "Ok",
    )
    .await
    .unwrap();
    assert_eq!(context.line, Some(1));
    post_line_comment(
        &store,
        &conn,
        API.0,
        API.1,
        20,
        &anchor("src/a.rs", 2, Side::Left, HEAD),
        "Removed",
    )
    .await
    .unwrap();
    let sent = writes(&api);
    let position = |i: usize| sent[i].2["position"].clone();
    assert_eq!(position(0)["new_line"], 3);
    assert!(position(0).get("old_line").is_none());
    assert_eq!(position(0)["head_sha"], HEAD);
    assert_eq!(position(0)["base_sha"], "b".repeat(40));
    assert_eq!(position(0)["start_sha"], "c".repeat(40));
    assert_eq!(
        (
            position(1)["old_line"].clone(),
            position(1)["new_line"].clone()
        ),
        (1.into(), 1.into())
    );
    assert_eq!(position(2)["old_line"], 2);
    assert!(position(2).get("new_line").is_none());
}

#[tokio::test]
async fn unsafe_line_comments_are_refused_before_anything_is_sent() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let missing = post_line_comment(
        &store,
        &conn,
        API.0,
        API.1,
        20,
        &anchor("src/a.rs", 99, Side::Right, HEAD),
        "x",
    )
    .await;
    assert!(
        matches!(missing, Err(GithubError::Validation(m)) if m.contains("n'est pas dans le diff"))
    );
    let other = post_line_comment(
        &store,
        &conn,
        API.0,
        API.1,
        20,
        &anchor("nope.rs", 1, Side::Right, HEAD),
        "x",
    )
    .await;
    assert!(matches!(other, Err(GithubError::Validation(_))));
    let mut range = anchor("src/a.rs", 3, Side::Right, HEAD);
    range.start_line = Some(2);
    let ranged = post_line_comment(&store, &conn, API.0, API.1, 20, &range, "x").await;
    assert!(matches!(ranged, Err(GithubError::Validation(m)) if m.contains("plage")));
    let old = "1".repeat(40);
    let stale = post_line_comment(
        &store,
        &conn,
        API.0,
        API.1,
        20,
        &anchor("src/a.rs", 3, Side::Right, &old),
        "x",
    )
    .await;
    assert!(matches!(stale, Err(GithubError::AnchorStale { .. })));
    assert!(writes(&api).is_empty());
    assert_eq!(
        current_head(&store, &conn, API.0, API.1, 20).await.unwrap(),
        HEAD
    );
}

#[tokio::test]
async fn a_reply_lands_in_the_discussion_of_the_comment_it_answers() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let reply = reply_to_comment(&store, &conn, API.0, API.1, 20, 602, "Thanks")
        .await
        .unwrap();
    assert_eq!(
        (reply.in_reply_to, reply.path.as_str()),
        (Some(601), "src/a.rs")
    );
    assert!(
        writes(&api)[0]
            .1
            .ends_with("/merge_requests/20/discussions/d1/notes")
    );
    let unknown = reply_to_comment(&store, &conn, API.0, API.1, 20, 9999, "x").await;
    assert!(matches!(unknown, Err(GithubError::NotFound)));
    assert_eq!(writes(&api).len(), 1);
}

#[tokio::test]
async fn an_approving_review_publishes_its_comments_in_one_step_after_the_approval() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let sent = submission(
        ReviewDecision::Approve,
        "Good overall",
        vec![line("src/a.rs", 3, Side::Right, "Nit here")],
        HEAD,
    );
    let review = submit_review(&store, &conn, API.0, API.1, 20, &sent)
        .await
        .unwrap();
    assert_eq!(
        (review.state.as_str(), review.author.as_str()),
        ("APPROVED", "Octo")
    );
    let steps: Vec<String> = writes(&api)
        .iter()
        .map(|(m, p, _)| format!("{m} {}", p.rsplit_once("/merge_requests/20/").unwrap().1))
        .collect();
    assert_eq!(
        steps,
        [
            "POST draft_notes",
            "POST draft_notes",
            "POST approve",
            "POST draft_notes/bulk_publish"
        ]
    );
    assert_eq!(writes(&api)[2].2["sha"], HEAD);
    let comments = list_comments(&store, &conn, API.0, API.1, 20)
        .await
        .unwrap();
    assert!(
        comments
            .review
            .iter()
            .any(|c| c.body == "Nit here" && c.author == "Octo")
    );
    let reviews = list_reviews(&store, &conn, API.0, API.1, 20).await.unwrap();
    assert!(review_exists(&reviews, "octo", &sent));
    let other = submission(ReviewDecision::Approve, "Different", Vec::new(), HEAD);
    assert!(!review_exists(&reviews, "octo", &other));
    let comment_only = submission(ReviewDecision::Comment, "", Vec::new(), HEAD);
    assert!(!review_exists(&reviews, "octo", &comment_only));
}

#[tokio::test]
async fn a_failure_while_preparing_the_review_leaves_nothing_published() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let sent = submission(
        ReviewDecision::Comment,
        "Summary",
        vec![
            line("src/a.rs", 3, Side::Right, "fine"),
            line("src/a.rs", 3, Side::Right, "boom"),
        ],
        HEAD,
    );
    let failed = submit_review(&store, &conn, API.0, API.1, 20, &sent).await;
    assert!(matches!(failed, Err(GithubError::Validation(_))));
    let calls = writes(&api);
    assert!(
        calls
            .iter()
            .any(|(m, p, _)| m == "DELETE" && p.contains("/draft_notes/"))
    );
    assert!(calls.iter().all(|(_, p, _)| !p.ends_with("bulk_publish")));
    let comments = list_comments(&store, &conn, API.0, API.1, 20)
        .await
        .unwrap();
    assert_eq!(comments.review.len(), 3);
}

#[tokio::test]
async fn a_refused_approval_withdraws_the_drafts_instead_of_publishing_half_a_review() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let sent = submission(ReviewDecision::Approve, "Mine", Vec::new(), &"8".repeat(40));
    let refused = submit_review(&store, &conn, API.0, API.1, 25, &sent).await;
    assert!(
        matches!(refused, Err(GithubError::Validation(m)) if m.contains("refuse l'approbation"))
    );
    let calls = writes(&api);
    assert!(calls.iter().any(|(m, _, _)| m == "DELETE"));
    assert!(calls.iter().all(|(_, p, _)| !p.ends_with("bulk_publish")));
}

#[tokio::test]
async fn decisions_gitlab_cannot_honour_and_stale_revisions_are_refused_without_a_request() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let changes = submission(ReviewDecision::RequestChanges, "Fix", Vec::new(), HEAD);
    assert!(matches!(
        submit_review(&store, &conn, API.0, API.1, 20, &changes).await,
        Err(GithubError::Validation(m)) if m.contains("demander des changements")
    ));
    let no_summary = submission(
        ReviewDecision::Comment,
        "",
        vec![line("src/a.rs", 3, Side::Right, "x")],
        HEAD,
    );
    assert!(matches!(
        submit_review(&store, &conn, API.0, API.1, 20, &no_summary).await,
        Err(GithubError::Validation(m)) if m.contains("ajoutez un message")
    ));
    let range = submission(
        ReviewDecision::Comment,
        "s",
        vec![DraftLine {
            start_line: Some(2),
            ..line("src/a.rs", 3, Side::Right, "x")
        }],
        HEAD,
    );
    assert!(matches!(
        submit_review(&store, &conn, API.0, API.1, 20, &range).await,
        Err(GithubError::Validation(_))
    ));
    let stale = submission(ReviewDecision::Approve, "s", Vec::new(), &"1".repeat(40));
    assert!(matches!(
        submit_review(&store, &conn, API.0, API.1, 20, &stale).await,
        Err(GithubError::AnchorStale { .. })
    ));
    assert!(writes(&api).is_empty());
}

#[tokio::test]
async fn pipeline_jobs_of_the_exact_revision_are_listed_with_their_real_states() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let checks = load_checks(&store, &conn, API.0, API.1, 20, HEAD)
        .await
        .unwrap();
    let states: Vec<(&str, CheckState)> = checks
        .items
        .iter()
        .map(|c| (c.name.as_str(), c.state))
        .collect();
    assert_eq!(
        states,
        [
            ("build", CheckState::Success),
            ("test", CheckState::Failure),
            ("lint", CheckState::Pending),
            ("deploy", CheckState::Cancelled),
            ("docs", CheckState::Skipped),
            ("e2e", CheckState::Pending),
        ]
    );
    assert_eq!(checks.overall(), Some(CheckState::Failure));
    let moved = load_checks(&store, &conn, API.0, API.1, 21, &"3".repeat(40))
        .await
        .unwrap();
    assert!(moved.items.is_empty());
    let fork = load_checks(&store, &conn, API.0, API.1, 34, &"e".repeat(40))
        .await
        .unwrap();
    assert_eq!(fork.items.len(), 1);
    assert!(api.requests().contains("/projects/200/pipelines/901/jobs"));
}
