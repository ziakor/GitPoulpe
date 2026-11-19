mod common;

use common::FakeApi;
use gitpoulpe_core::Provider;
use gitpoulpe_hosting::{
    GithubError, GitlabHost, MemoryStore, SecretStore, check_connection, connect_gitlab,
};

fn host(api: &FakeApi) -> GitlabHost {
    GitlabHost {
        api: format!("{}/api/v4", api.base()),
        web: api.base(),
    }
}

#[tokio::test]
async fn connecting_shows_the_account_the_granted_scopes_and_the_expiry_and_keeps_the_token_in_the_store()
 {
    let api = FakeApi::start("fake_gitlab.py");
    let store = MemoryStore::default();
    let (connection, session) = connect_gitlab(&store, &host(&api), " glgood\n")
        .await
        .unwrap();
    assert_eq!(session.login, "Octo");
    assert_eq!(
        session.scopes,
        ["read_api", "read_repository", "write_repository"]
    );
    assert_eq!(session.expires_at.as_deref(), Some("2027-01-31"));
    assert_eq!(connection.provider, Provider::Gitlab);
    assert_eq!(connection.secret_key(), format!("gitlab:{}", connection.id));
    assert_eq!(
        store.get(&connection.secret_key()).unwrap().as_deref(),
        Some("glgood")
    );
    let json = serde_json::to_string(&connection).unwrap();
    assert!(!json.contains("glgood"), "{json}");
    assert!(api.requests().contains("\"auth\": \"Bearer glgood\""));
    let checked = check_connection(&store, &connection).await.unwrap();
    assert_eq!(checked.login, "Octo");
}

#[tokio::test]
async fn a_token_without_expiry_and_an_older_server_without_token_details_are_still_connectable() {
    let api = FakeApi::start("fake_gitlab.py");
    let store = MemoryStore::default();
    let (_, none) = connect_gitlab(&store, &host(&api), "glnoexp")
        .await
        .unwrap();
    assert_eq!(none.expires_at, None);
    assert_eq!(none.scopes, ["api"]);
    let (_, old) = connect_gitlab(&store, &host(&api), "glold").await.unwrap();
    assert!(old.scopes.is_empty() && old.expires_at.is_none());
}

#[tokio::test]
async fn expired_blocked_and_limited_tokens_have_distinct_states_and_nothing_is_stored() {
    let api = FakeApi::start("fake_gitlab.py");
    let store = MemoryStore::default();
    assert_eq!(
        connect_gitlab(&store, &host(&api), "gone")
            .await
            .unwrap_err(),
        GithubError::Unauthorized
    );
    assert!(matches!(
        connect_gitlab(&store, &host(&api), "glforbid").await.unwrap_err(),
        GithubError::Forbidden(m) if m.contains("blocked")
    ));
    let limited = connect_gitlab(&store, &host(&api), "glrate")
        .await
        .unwrap_err();
    assert_eq!(limited, GithubError::RateLimited(Some("30".to_owned())));
    assert!(limited.to_string().contains("indication du serveur : 30"));
    assert_eq!(
        connect_gitlab(&store, &host(&api), "  ").await.unwrap_err(),
        GithubError::EmptyToken
    );
    assert!(store.keys().is_empty());
}

#[tokio::test]
async fn a_token_that_belongs_to_another_account_or_was_revoked_is_reported_on_the_next_check() {
    let api = FakeApi::start("fake_gitlab.py");
    let store = MemoryStore::default();
    let (connection, _) = connect_gitlab(&store, &host(&api), "glgood").await.unwrap();
    store.set(&connection.secret_key(), "glother").unwrap();
    assert!(matches!(
        check_connection(&store, &connection).await.unwrap_err(),
        GithubError::Forbidden(_)
    ));
    store.set(&connection.secret_key(), "revoked").unwrap();
    assert_eq!(
        check_connection(&store, &connection).await.unwrap_err(),
        GithubError::Unauthorized
    );
    store.delete(&connection.secret_key()).unwrap();
    assert_eq!(
        check_connection(&store, &connection).await.unwrap_err(),
        GithubError::NotConnected
    );
}

#[test]
fn self_managed_instances_follow_the_https_rule_and_get_the_v4_prefix() {
    assert_eq!(GitlabHost::dotcom().api, "https://gitlab.com/api/v4");
    assert_eq!(
        GitlabHost::self_managed("https://git.example.org/")
            .unwrap()
            .api,
        "https://git.example.org/api/v4"
    );
    assert!(GitlabHost::self_managed("http://git.example.org").is_err());
    assert!(GitlabHost::self_managed("http://127.0.0.1:8080").is_ok());
}
