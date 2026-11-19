mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{CheckState, GithubError, MemoryStore, SecretStore, load_checks};

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

#[tokio::test]
async fn pending_success_failure_cancelled_and_skipped_checks_are_told_apart_across_pages() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let checks = load_checks(&store, &conn, "acme", "proj", 1, "sha1")
        .await
        .unwrap();
    assert_eq!(checks.sha, "sha1");
    let state = |name: &str| checks.items.iter().find(|c| c.name == name).unwrap().state;
    assert_eq!(state("build"), CheckState::Success);
    assert_eq!(state("lint"), CheckState::Failure);
    assert_eq!(state("deploy"), CheckState::Pending);
    assert_eq!(state("e2e"), CheckState::Cancelled);
    assert_eq!(state("docs"), CheckState::Skipped);
    assert_eq!(state("legacy/ok"), CheckState::Success);
    assert!(
        checks
            .items
            .iter()
            .find(|c| c.name == "legacy/ok")
            .unwrap()
            .legacy
    );
    assert_eq!(checks.items.len(), 6);
    assert_eq!(checks.overall(), Some(CheckState::Failure));
    assert_eq!(checks.count(CheckState::Pending), 1);
    assert_eq!(checks.items[0].url, "https://ci.test/1");
}

#[tokio::test]
async fn a_revision_without_checks_is_absent_not_green_and_a_legacy_status_is_read() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let none = load_checks(&store, &conn, "acme", "proj", 1, "newsha")
        .await
        .unwrap();
    assert!(none.items.is_empty());
    assert_eq!(none.overall(), None);
    let legacy = load_checks(&store, &conn, "acme", "proj", 1, "sha2")
        .await
        .unwrap();
    assert_eq!(legacy.overall(), Some(CheckState::Pending));
    assert_eq!(legacy.items[0].name, "legacy/ci");
}

#[tokio::test]
async fn an_unavailable_api_is_an_error_never_an_empty_or_successful_list() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    assert_eq!(
        load_checks(&store, &conn, "acme", "proj", 1, "sha5")
            .await
            .unwrap_err(),
        GithubError::Server(500)
    );
    for bad in ["", "../x", "a b", "sha1/status"] {
        assert_eq!(
            load_checks(&store, &conn, "acme", "proj", 1, bad)
                .await
                .unwrap_err(),
            GithubError::NotFound
        );
    }
}
