mod common;

use std::cell::Cell;
use std::time::Duration;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{GithubError, MemoryStore, SecretStore, create_fork};

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

fn posts(api: &FakeApi) -> usize {
    api.requests().matches("/forks").count()
}

#[tokio::test]
async fn a_personal_fork_is_created_waited_for_and_traced_back_to_its_source() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let pauses = Cell::new(0);
    let outcome = create_fork(&store, &conn, "acme", "tool", None, |_: Duration| {
        pauses.set(pauses.get() + 1);
        async {}
    })
    .await
    .unwrap();
    assert!(!outcome.existed);
    assert_eq!(outcome.repo.full_name(), "octo/tool");
    assert_eq!(outcome.repo.parent.as_deref(), Some("acme/tool"));
    assert_eq!(posts(&api), 1);
}

#[tokio::test]
async fn an_organization_destination_is_used_and_an_existing_fork_creates_no_duplicate() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let org = create_fork(&store, &conn, "acme", "tool", Some("zeta"), |_| async {})
        .await
        .unwrap();
    assert_eq!(org.repo.full_name(), "zeta/tool");
    assert!(api.requests().contains("\\\"organization\\\":\\\"zeta\\\""));
    let before = posts(&api);
    let again = create_fork(&store, &conn, "acme", "existing", None, |_| async {})
        .await
        .unwrap();
    assert!(again.existed);
    assert_eq!(again.repo.full_name(), "octo/existing");
    assert_eq!(posts(&api), before);
}

#[tokio::test]
async fn a_name_taken_by_an_unrelated_repository_and_a_refused_destination_change_nothing() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    assert_eq!(
        create_fork(&store, &conn, "acme", "taken", None, |_| async {})
            .await
            .unwrap_err(),
        GithubError::NameTaken("octo/taken".to_owned())
    );
    assert_eq!(posts(&api), 0);
    let error = create_fork(&store, &conn, "acme", "tool", Some("locked"), |_| async {})
        .await
        .unwrap_err();
    assert!(matches!(error, GithubError::Forbidden(_)), "{error:?}");
    let before = api.requests().lines().count();
    for bad in ["../x", "a/b", ""] {
        assert_eq!(
            create_fork(&store, &conn, "acme", "tool", Some(bad), |_| async {})
                .await
                .unwrap_err(),
            GithubError::NotFound
        );
    }
    assert_eq!(api.requests().lines().count(), before);
}

#[tokio::test]
async fn a_fork_that_never_becomes_ready_is_reported_as_pending_without_a_second_request_to_create_it()
 {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let error = create_fork(&store, &conn, "acme", "never", None, |_| async {})
        .await
        .unwrap_err();
    assert_eq!(error, GithubError::ForkPending);
    assert_eq!(posts(&api), 1);
}
