mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{
    DraftLine, GithubError, MemoryStore, ReviewDecision, ReviewSubmission, SecretStore, Side,
    list_reviews, review_exists, submit_review,
};

fn connection(api: &FakeApi) -> Connection {
    Connection {
        id: "one/octo".to_owned(),
        provider: Provider::Github,
        api: api.base(),
        web: api.base(),
        login: "Octo".to_owned(),
        scopes: Vec::new(),
    }
}

fn stored(connection: &Connection) -> MemoryStore {
    let store = MemoryStore::default();
    store.set(&connection.secret_key(), "good").unwrap();
    store
}

fn line(path: &str, n: u64, start: Option<u64>, side: Side, body: &str) -> DraftLine {
    DraftLine {
        path: path.to_owned(),
        line: n,
        start_line: start,
        side,
        body: body.to_owned(),
    }
}

fn submission(sha: &str, decision: ReviewDecision, body: &str) -> ReviewSubmission {
    ReviewSubmission {
        commit_sha: sha.to_owned(),
        body: body.to_owned(),
        decision,
        lines: vec![
            line("f1.rs", 3, Some(1), Side::Right, " a "),
            line("f2.rs", 2, None, Side::Left, "b"),
        ],
    }
}

#[tokio::test]
async fn one_review_carries_every_comment_the_decision_and_the_revision_and_can_be_read_back() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let sent = submission("sha1", ReviewDecision::RequestChanges, " fix these ");
    let review = submit_review(&store, &conn, "acme", "proj", 1, &sent)
        .await
        .unwrap();
    assert_eq!(review.state, "CHANGES_REQUESTED");
    assert_eq!(
        (review.author.as_str(), review.commit_sha.as_str()),
        ("Octo", "sha1")
    );
    assert_eq!(review.body, "fix these");
    let log = api.requests();
    assert!(
        log.contains("\\\"event\\\":\\\"REQUEST_CHANGES\\\""),
        "{log}"
    );
    assert_eq!(log.matches("reviews\", \"auth\"").count(), 1, "{log}");
    assert!(
        log.contains("\\\"start_line\\\":1") && log.contains("\\\"side\\\":\\\"LEFT\\\""),
        "{log}"
    );
    let listed = list_reviews(&store, &conn, "acme", "proj", 1)
        .await
        .unwrap();
    assert!(
        listed
            .iter()
            .any(|r| r.id == review.id && r.state == "CHANGES_REQUESTED")
    );
    assert!(review_exists(&listed, "octo", &sent));
    assert!(!review_exists(&listed, "someone", &sent));
    assert!(!review_exists(
        &listed,
        "octo",
        &submission("other", ReviewDecision::Comment, "fix these")
    ));
}

#[tokio::test]
async fn the_three_decisions_are_told_apart() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    for (decision, state) in [
        (ReviewDecision::Comment, "COMMENTED"),
        (ReviewDecision::Approve, "APPROVED"),
        (ReviewDecision::RequestChanges, "CHANGES_REQUESTED"),
    ] {
        let review = submit_review(
            &store,
            &conn,
            "acme",
            "proj",
            1,
            &submission("sha1", decision, "msg"),
        )
        .await
        .unwrap();
        assert_eq!(review.state, state);
    }
}

#[tokio::test]
async fn an_obsolete_revision_missing_permission_and_invalid_drafts_never_look_successful() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let stale = submit_review(
        &store,
        &conn,
        "acme",
        "proj",
        4,
        &submission("oldsha", ReviewDecision::Approve, "x"),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(stale, GithubError::AnchorStale { .. }),
        "{stale:?}"
    );
    let own = submit_review(
        &store,
        &conn,
        "acme",
        "proj",
        1,
        &submission("sha1", ReviewDecision::Approve, "self"),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&own, GithubError::Validation(m) if m.contains("your own pull request")),
        "{own:?}"
    );
    let before = api.requests().matches("reviews\", \"auth\"").count();
    for bad in [
        submission("sha1", ReviewDecision::RequestChanges, "  "),
        ReviewSubmission {
            lines: vec![],
            ..submission("sha1", ReviewDecision::Comment, " ")
        },
        ReviewSubmission {
            lines: vec![line("f", 0, None, Side::Right, "x")],
            ..submission("sha1", ReviewDecision::Comment, "m")
        },
        ReviewSubmission {
            lines: vec![line("f", 2, Some(5), Side::Right, "x")],
            ..submission("sha1", ReviewDecision::Comment, "m")
        },
        ReviewSubmission {
            lines: vec![line("f", 2, None, Side::Right, " ")],
            ..submission("sha1", ReviewDecision::Comment, "m")
        },
    ] {
        assert!(matches!(
            submit_review(&store, &conn, "acme", "proj", 1, &bad)
                .await
                .unwrap_err(),
            GithubError::Validation(_)
        ));
    }
    assert_eq!(
        api.requests().matches("reviews\", \"auth\"").count(),
        before
    );
    assert_eq!(
        submit_review(
            &store,
            &conn,
            "../x",
            "proj",
            1,
            &submission("sha1", ReviewDecision::Comment, "m")
        )
        .await
        .unwrap_err(),
        GithubError::NotFound
    );
}

#[tokio::test]
async fn a_lost_answer_leaves_the_review_findable_so_it_is_not_sent_twice() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let sent = submission("sha1", ReviewDecision::Comment, "FLAKY thoughts");
    assert_eq!(
        submit_review(&store, &conn, "acme", "proj", 1, &sent)
            .await
            .unwrap_err(),
        GithubError::Server(502)
    );
    let listed = list_reviews(&store, &conn, "acme", "proj", 1)
        .await
        .unwrap();
    assert!(review_exists(&listed, "Octo", &sent));
}
