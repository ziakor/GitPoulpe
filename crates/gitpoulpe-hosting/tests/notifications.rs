mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, NoticeKind, Provider};
use gitpoulpe_hosting::{GithubError, MemoryStore, SecretStore, fetch_notifications};

fn github(api: &FakeApi) -> Connection {
    Connection {
        id: "gh/octo".to_owned(),
        provider: Provider::Github,
        api: api.base(),
        web: api.base(),
        login: "Octo".to_owned(),
        scopes: Vec::new(),
    }
}

fn gitlab(api: &FakeApi) -> Connection {
    Connection {
        id: "gl/octo".to_owned(),
        provider: Provider::Gitlab,
        api: format!("{}/api/v4", api.base()),
        web: api.base(),
        login: "Octo".to_owned(),
        scopes: Vec::new(),
    }
}

fn stored(connection: &Connection, token: &str) -> MemoryStore {
    let store = MemoryStore::default();
    store.set(&connection.secret_key(), token).unwrap();
    store
}

#[tokio::test]
async fn github_notifications_keep_only_unread_ones_with_their_kind_and_pull_number() {
    let api = FakeApi::start("fake_github.py");
    let conn = github(&api);
    let found = fetch_notifications(&stored(&conn, "good"), &conn)
        .await
        .unwrap();
    let summary: Vec<(&str, NoticeKind, Option<u64>)> = found
        .iter()
        .map(|n| (n.id.as_str(), n.kind, n.number))
        .collect();
    assert_eq!(
        summary,
        [
            ("n1", NoticeKind::Review, Some(1)),
            ("n2", NoticeKind::Checks, None),
            ("n4", NoticeKind::Mention, Some(12)),
        ]
    );
    assert_eq!(found[0].repo.as_deref(), Some("acme/proj"));
    assert_eq!(found[0].title, "PR 1");
    assert!(api.requests().contains("\"auth\": \"Bearer good\""));
}

#[tokio::test]
async fn gitlab_todos_become_notifications_with_their_merge_request_number() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = gitlab(&api);
    let found = fetch_notifications(&stored(&conn, "glgood"), &conn)
        .await
        .unwrap();
    let summary: Vec<(&str, NoticeKind, Option<u64>)> = found
        .iter()
        .map(|n| (n.id.as_str(), n.kind, n.number))
        .collect();
    assert_eq!(
        summary,
        [
            ("11", NoticeKind::Review, Some(5)),
            ("12", NoticeKind::Checks, Some(5)),
            ("13", NoticeKind::Mention, Some(9)),
            ("14", NoticeKind::Activity, Some(3)),
        ]
    );
    assert_eq!(found[3].repo.as_deref(), Some("octo/notes"));
}

#[tokio::test]
async fn a_revoked_token_or_a_missing_one_is_an_error_never_an_empty_inbox() {
    let api = FakeApi::start("fake_github.py");
    let conn = github(&api);
    assert!(matches!(
        fetch_notifications(&stored(&conn, "bad"), &conn).await,
        Err(GithubError::Unauthorized)
    ));
    assert!(matches!(
        fetch_notifications(&MemoryStore::default(), &conn).await,
        Err(GithubError::NotConnected)
    ));
}
