mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{
    GithubError, LineAnchor, MemoryStore, SecretStore, Side, list_comments, post_comment,
    post_line_comment, reply_to_comment,
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

fn anchor(sha: &str, line: u64, start: Option<u64>, side: Side) -> LineAnchor {
    LineAnchor {
        commit_sha: sha.to_owned(),
        path: "f1.rs".to_owned(),
        line,
        start_line: start,
        side,
    }
}

#[tokio::test]
async fn general_and_line_comments_are_read_with_their_threads_and_outdated_marks() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let all = list_comments(&store, &conn, "acme", "proj", 1)
        .await
        .unwrap();
    assert_eq!(all.issue.len(), 1);
    assert_eq!(all.review.len(), 3);
    let reply = all.review.iter().find(|c| c.id == 11).unwrap();
    assert_eq!(reply.in_reply_to, Some(10));
    let old = all.review.iter().find(|c| c.id == 12).unwrap();
    assert!(old.outdated);
    assert_eq!((old.line, old.side), (Some(3), Side::Left));
    assert!(!all.review[0].outdated);
}

#[tokio::test]
async fn a_line_comment_carries_the_revision_side_and_range_actually_displayed() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let added = post_line_comment(
        &store,
        &conn,
        "acme",
        "proj",
        1,
        &anchor("sha1", 5, Some(3), Side::Right),
        " nit ",
    )
    .await
    .unwrap();
    assert_eq!(
        (added.line, added.side, added.body.as_str()),
        (Some(5), Side::Right, "nit")
    );
    let removed = post_line_comment(
        &store,
        &conn,
        "acme",
        "proj",
        1,
        &anchor("sha1", 2, None, Side::Left),
        "gone",
    )
    .await
    .unwrap();
    assert_eq!(removed.side, Side::Left);
    let log = api.requests();
    assert!(log.contains("\\\"commit_id\\\":\\\"sha1\\\""), "{log}");
    assert!(
        log.contains("\\\"side\\\":\\\"RIGHT\\\"") && log.contains("\\\"start_line\\\":3"),
        "{log}"
    );
    assert!(log.contains("\\\"side\\\":\\\"LEFT\\\""), "{log}");
}

#[tokio::test]
async fn a_stale_anchor_an_empty_text_and_a_bad_range_are_refused_before_anything_is_sent() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let error = post_line_comment(
        &store,
        &conn,
        "acme",
        "proj",
        4,
        &anchor("oldsha", 1, None, Side::Right),
        "x",
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, GithubError::AnchorStale { .. }),
        "{error:?}"
    );
    for (body, a) in [
        ("  ", anchor("sha1", 1, None, Side::Right)),
        ("x", anchor("sha1", 0, None, Side::Right)),
        ("x", anchor("sha1", 2, Some(5), Side::Right)),
    ] {
        assert!(matches!(
            post_line_comment(&store, &conn, "acme", "proj", 1, &a, body)
                .await
                .unwrap_err(),
            GithubError::Validation(_)
        ));
    }
    assert_eq!(
        api.requests().matches("/comments\"").count(),
        0,
        "{}",
        api.requests()
    );
}

#[tokio::test]
async fn replies_and_general_comments_are_posted_and_a_lost_response_can_be_verified_before_retrying()
 {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let reply = reply_to_comment(&store, &conn, "acme", "proj", 1, 10, "thanks")
        .await
        .unwrap();
    assert_eq!(reply.in_reply_to, Some(10));
    let general = post_comment(&store, &conn, "acme", "proj", 1, "LGTM")
        .await
        .unwrap();
    assert_eq!(general.body, "LGTM");
    let lost = post_comment(&store, &conn, "acme", "proj", 1, "FLAKY note")
        .await
        .unwrap_err();
    assert_eq!(lost, GithubError::Server(500));
    let after = list_comments(&store, &conn, "acme", "proj", 1)
        .await
        .unwrap();
    assert!(after.contains("octo", "FLAKY note"));
    assert!(after.contains("Octo", " FLAKY note "));
    assert!(!after.contains("octo", "never sent"));
    assert!(!after.contains("someone", "FLAKY note"));
}
