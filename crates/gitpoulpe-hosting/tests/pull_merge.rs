mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{
    CheckState, GithubError, MemoryStore, MergeMethod, MergeRequest, PullState, SecretStore,
    is_merged, merge_pull, prepare_merge,
};

fn connection(api: &FakeApi) -> Connection {
    Connection {
        id: "one/octo".to_owned(),
        provider: Provider::Github,
        api: api.base(),
        web: api.base(),
        login: "octo".to_owned(),
        scopes: Vec::new(),
    }
}

fn stored(connection: &Connection) -> MemoryStore {
    let store = MemoryStore::default();
    store.set(&connection.secret_key(), "good").unwrap();
    store
}

fn request(method: MergeMethod, head: &str, message: &str) -> MergeRequest {
    MergeRequest {
        method,
        expected_head: head.to_owned(),
        message: message.to_owned(),
    }
}

fn puts(api: &FakeApi) -> usize {
    api.requests().matches("\"put\": true").count()
}

#[tokio::test]
async fn only_the_methods_the_repository_allows_are_offered_with_protections_and_blockers_explained()
 {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let ready = prepare_merge(&store, &conn, "acme", "proj", 1)
        .await
        .unwrap();
    assert_eq!(ready.methods, [MergeMethod::Merge, MergeMethod::Squash]);
    assert_eq!(ready.state, PullState::Open);
    assert_eq!(ready.head_sha, "sha1");
    assert_eq!((ready.approvals, ready.required_approvals), (1, 2));
    let state = |name: &str| {
        ready
            .required_checks
            .iter()
            .find(|c| c.name == name)
            .unwrap()
            .state
    };
    assert_eq!(state("build"), Some(CheckState::Success));
    assert_eq!(state("lint"), Some(CheckState::Failure));
    assert_eq!(state("missing"), None);
    let all = ready.blockers.join(" | ");
    assert!(all.contains("« lint » n'a pas réussi"), "{all}");
    assert!(all.contains("« missing » est absente"), "{all}");
    assert!(all.contains("1 approbation sur 2 requises"), "{all}");
    assert!(!all.contains("« build »"), "{all}");
    assert!(ready.can_attempt());
}

#[tokio::test]
async fn a_blocked_a_closed_and_a_draft_pull_request_are_explained_and_cannot_be_attempted() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let blocked = prepare_merge(&store, &conn, "acme", "proj", 4)
        .await
        .unwrap();
    assert!(blocked.draft && !blocked.can_attempt());
    let all = blocked.blockers.join(" | ");
    assert!(
        all.contains("brouillon") && all.contains("protections de branche"),
        "{all}"
    );
    let closed = prepare_merge(&store, &conn, "acme", "proj", 2)
        .await
        .unwrap();
    assert_eq!(closed.state, PullState::Closed);
    assert!(
        closed
            .blockers
            .iter()
            .any(|b| b.contains("n'est plus ouverte"))
    );
    assert!(!closed.can_attempt());
}

#[tokio::test]
async fn merging_sends_the_expected_revision_and_verifies_the_remote_result() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let outcome = merge_pull(
        &store,
        &conn,
        "acme",
        "proj",
        1,
        &request(MergeMethod::Squash, "sha1", "msg"),
    )
    .await
    .unwrap();
    assert_eq!(outcome.sha, "mergesha");
    assert!(outcome.verified_merged);
    let log = api.requests();
    assert!(log.contains("\"merge_method\": \"squash\""), "{log}");
    assert!(log.contains("\"sha\": \"sha1\""), "{log}");
    assert!(is_merged(&store, &conn, "acme", "proj", 1).await.unwrap());
    let again = prepare_merge(&store, &conn, "acme", "proj", 1)
        .await
        .unwrap();
    assert_eq!(again.state, PullState::Merged);
}

#[tokio::test]
async fn an_advanced_revision_and_a_server_refusal_never_look_like_a_merge() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let stale = merge_pull(
        &store,
        &conn,
        "acme",
        "proj",
        1,
        &request(MergeMethod::Merge, "oldsha0", ""),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(stale, GithubError::AnchorStale { .. }),
        "{stale:?}"
    );
    assert_eq!(puts(&api), 0);
    let refused = merge_pull(
        &store,
        &conn,
        "acme",
        "proj",
        4,
        &request(MergeMethod::Merge, "newsha", ""),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&refused, GithubError::MergeRefused(m) if m.contains("lint")),
        "{refused:?}"
    );
    assert!(!is_merged(&store, &conn, "acme", "proj", 4).await.unwrap());
    assert_eq!(
        merge_pull(
            &store,
            &conn,
            "../x",
            "proj",
            1,
            &request(MergeMethod::Merge, "sha1", "")
        )
        .await
        .unwrap_err(),
        GithubError::NotFound
    );
}

#[tokio::test]
async fn a_lost_answer_is_resolved_by_reading_the_remote_state() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    assert!(!is_merged(&store, &conn, "acme", "proj", 1).await.unwrap());
    let lost = merge_pull(
        &store,
        &conn,
        "acme",
        "proj",
        1,
        &request(MergeMethod::Merge, "sha1", "FLAKY"),
    )
    .await
    .unwrap_err();
    assert_eq!(lost, GithubError::Server(502));
    assert!(is_merged(&store, &conn, "acme", "proj", 1).await.unwrap());
}

#[tokio::test]
async fn a_success_answer_the_remote_does_not_confirm_is_not_reported_as_verified() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let outcome = merge_pull(
        &store,
        &conn,
        "acme",
        "proj",
        1,
        &request(MergeMethod::Merge, "sha1", "GHOST"),
    )
    .await
    .unwrap();
    assert!(!outcome.verified_merged);
}

#[tokio::test]
async fn requested_changes_from_a_reviewer_are_listed_as_a_blocker() {
    use gitpoulpe_hosting::{DraftLine, ReviewDecision, ReviewSubmission, submit_review};
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let before = prepare_merge(&store, &conn, "acme", "proj", 1)
        .await
        .unwrap();
    assert!(
        !before
            .blockers
            .iter()
            .any(|b| b.contains("changements ont été demandés"))
    );
    submit_review(
        &store,
        &conn,
        "acme",
        "proj",
        1,
        &ReviewSubmission {
            commit_sha: "sha1".to_owned(),
            body: "no".to_owned(),
            decision: ReviewDecision::RequestChanges,
            lines: Vec::<DraftLine>::new(),
        },
    )
    .await
    .unwrap();
    let after = prepare_merge(&store, &conn, "acme", "proj", 1)
        .await
        .unwrap();
    assert!(
        after
            .blockers
            .iter()
            .any(|b| b.contains("changements ont été demandés"))
    );
}
