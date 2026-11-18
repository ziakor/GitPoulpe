mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{
    GithubError, MemoryStore, SecretStore, list_organizations, list_repositories,
};

fn connection(api: &FakeApi, id: &str) -> Connection {
    Connection {
        id: id.to_owned(),
        provider: Provider::Github,
        api: api.base(),
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
async fn personal_repositories_are_collected_across_pages_sorted_and_tagged_with_their_connection()
{
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api, "one/octo");
    let store = stored(&conn, "good");
    let listing = list_repositories(&store, &conn, None).await.unwrap();
    assert!(!listing.truncated);
    let names: Vec<String> = listing.items.iter().map(|r| r.full_name()).collect();
    assert_eq!(names, ["acme/Zed", "octo/a", "octo/b"]);
    let b = &listing.items[2];
    assert!(b.private && !b.fork && b.can_push);
    assert_eq!(b.description.as_deref(), Some("secret"));
    assert_eq!(b.default_branch.as_deref(), Some("main"));
    assert_eq!(b.clone_url, "https://example.test/octo/b.git");
    assert!(listing.items[0].fork);
    assert!(listing.items.iter().all(|r| r.connection_id == "one/octo"));
    assert!(api.requests().contains("page=2"));
}

#[tokio::test]
async fn organizations_and_their_repositories_keep_archived_and_read_only_flags() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api, "one/octo");
    let store = stored(&conn, "good");
    let orgs = list_organizations(&store, &conn).await.unwrap();
    let logins: Vec<&str> = orgs.items.iter().map(|o| o.login.as_str()).collect();
    assert_eq!(logins, ["acme", "Zeta"]);
    let repos = list_repositories(&store, &conn, Some("acme"))
        .await
        .unwrap();
    assert_eq!(repos.items.len(), 1);
    assert!(repos.items[0].archived && !repos.items[0].can_push);
    assert!(api.requests().contains("/orgs/acme/repos?type=all"));
}

#[tokio::test]
async fn failures_keep_their_distinct_state_and_a_bad_organization_name_never_reaches_the_network()
{
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api, "one/octo");
    assert!(matches!(
        list_repositories(&stored(&conn, "sso"), &conn, None)
            .await
            .unwrap_err(),
        GithubError::SsoRequired(_)
    ));
    assert_eq!(
        list_organizations(&stored(&conn, "bad"), &conn)
            .await
            .unwrap_err(),
        GithubError::Unauthorized
    );
    assert_eq!(
        list_repositories(&MemoryStore::default(), &conn, None)
            .await
            .unwrap_err(),
        GithubError::NotConnected
    );
    let good = stored(&conn, "good");
    assert_eq!(
        list_repositories(&good, &conn, Some("ghost"))
            .await
            .unwrap_err(),
        GithubError::NotFound
    );
    let before = api.requests();
    for bad in ["", "../user", "a/b", "a?x=1", "a b"] {
        assert_eq!(
            list_repositories(&good, &conn, Some(bad))
                .await
                .unwrap_err(),
            GithubError::NotFound,
            "{bad:?}"
        );
    }
    assert_eq!(api.requests(), before);
}

#[tokio::test]
async fn two_connections_to_different_servers_never_share_results_or_tokens() {
    let first = FakeApi::start("fake_github.py");
    let second = FakeApi::start("fake_github.py");
    let a = connection(&first, "one/octo");
    let b = connection(&second, "two/octo");
    let store = MemoryStore::default();
    store.set(&a.secret_key(), "good").unwrap();
    store.set(&b.secret_key(), "good").unwrap();
    let from_a = list_repositories(&store, &a, Some("acme")).await.unwrap();
    let from_b = list_repositories(&store, &b, Some("acme")).await.unwrap();
    assert_eq!(from_a.items[0].connection_id, "one/octo");
    assert_eq!(from_b.items[0].connection_id, "two/octo");
    assert_eq!(first.requests().lines().count(), 1);
    assert_eq!(second.requests().lines().count(), 1);
}

#[tokio::test]
async fn a_next_page_pointing_to_another_server_is_refused_and_receives_no_request() {
    let api = FakeApi::start("fake_github.py");
    let other = FakeApi::start("fake_github.py");
    let conn = connection(&api, "one/octo");
    let store = stored(&conn, "evil");
    let error = list_repositories(&store, &conn, None).await.unwrap_err();
    assert!(matches!(error, GithubError::Http(_)), "{error:?}");
    assert!(other.requests().is_empty());
}
