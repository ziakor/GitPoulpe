mod common;

use common::FakeApi;
use gitpoulpe_core::{ConnectionBook, Provider};
use gitpoulpe_hosting::{
    ApiClient, GithubError, GithubHost, HttpError, MemoryStore, SecretStore, Token,
    check_connection, connect_github, disconnect, validate_base,
};

fn host(api: &FakeApi) -> GithubHost {
    GithubHost {
        api: api.base(),
        web: api.base(),
    }
}

#[tokio::test]
async fn connecting_shows_the_account_and_only_the_granted_scopes_and_stores_the_token_in_the_store()
 {
    let api = FakeApi::start("fake_github.py");
    let store = MemoryStore::default();
    let (connection, session) = connect_github(&store, &host(&api), " good \n")
        .await
        .unwrap();
    assert_eq!(session.login, "Octo");
    assert_eq!(session.name.as_deref(), Some("Octo Cat"));
    assert_eq!(session.scopes, ["repo", "read:org"]);
    assert!(!session.fine_grained);
    assert_eq!(connection.provider, Provider::Github);
    assert_eq!(connection.scopes, session.scopes);
    assert_eq!(
        store.get(&connection.secret_key()).unwrap().as_deref(),
        Some("good")
    );
    let book = serde_json::to_string(&ConnectionBook {
        connections: vec![connection],
    })
    .unwrap();
    assert!(!book.contains("good\""), "{book}");
    assert!(api.requests().contains("\"auth\": \"Bearer good\""));
}

#[tokio::test]
async fn a_fine_grained_token_is_recognised_by_its_missing_scope_header() {
    let api = FakeApi::start("fake_github.py");
    let store = MemoryStore::default();
    let (_, session) = connect_github(&store, &host(&api), "fine").await.unwrap();
    assert!(session.fine_grained);
    assert!(session.scopes.is_empty());
}

#[tokio::test]
async fn expired_sso_restricted_limited_and_denied_tokens_have_distinct_recoverable_states_and_are_not_stored()
 {
    let api = FakeApi::start("fake_github.py");
    let store = MemoryStore::default();
    assert_eq!(
        connect_github(&store, &host(&api), "revoked")
            .await
            .unwrap_err(),
        GithubError::Unauthorized
    );
    match connect_github(&store, &host(&api), "sso")
        .await
        .unwrap_err()
    {
        GithubError::SsoRequired(url) => assert!(url.contains("orgs/acme/sso"), "{url}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        connect_github(&store, &host(&api), "limited")
            .await
            .unwrap_err(),
        GithubError::RateLimited(Some("1893456000".to_owned()))
    );
    assert!(matches!(
        connect_github(&store, &host(&api), "denied").await.unwrap_err(),
        GithubError::Forbidden(m) if m.contains("not accessible")
    ));
    assert_eq!(
        connect_github(&store, &host(&api), "  ").await.unwrap_err(),
        GithubError::EmptyToken
    );
    assert!(store.keys().is_empty());
}

#[tokio::test]
async fn a_connection_whose_token_was_revoked_is_reported_and_can_be_reconnected() {
    let api = FakeApi::start("fake_github.py");
    let store = MemoryStore::default();
    let (connection, _) = connect_github(&store, &host(&api), "good").await.unwrap();
    assert_eq!(
        check_connection(&store, &connection).await.unwrap().login,
        "Octo"
    );
    store.set(&connection.secret_key(), "revoked").unwrap();
    assert_eq!(
        check_connection(&store, &connection).await.unwrap_err(),
        GithubError::Unauthorized
    );
    store.set(&connection.secret_key(), "other").unwrap();
    assert!(matches!(
        check_connection(&store, &connection).await.unwrap_err(),
        GithubError::Forbidden(_)
    ));
    let (again, _) = connect_github(&store, &host(&api), "good").await.unwrap();
    assert_eq!(again.id, connection.id);
    assert_eq!(
        check_connection(&store, &again).await.unwrap().login,
        "Octo"
    );
}

#[tokio::test]
async fn disconnecting_removes_only_that_connections_credential() {
    let api = FakeApi::start("fake_github.py");
    let store = MemoryStore::default();
    let (octo, _) = connect_github(&store, &host(&api), "good").await.unwrap();
    store.set("unrelated", "keep").unwrap();
    disconnect(&store, &octo).unwrap();
    assert_eq!(store.keys(), ["unrelated"]);
    assert_eq!(
        check_connection(&store, &octo).await.unwrap_err(),
        GithubError::NotConnected
    );
}

#[tokio::test]
async fn the_token_never_appears_in_errors_debug_output_or_after_a_redirect() {
    let api = FakeApi::start("fake_github.py");
    let secret = "ghp_supersecretvalue1234567890";
    let store = MemoryStore::default();
    let error = connect_github(&store, &host(&api), secret)
        .await
        .unwrap_err();
    assert!(!format!("{error} {error:?}").contains(secret));
    assert_eq!(format!("{:?}", Token::new(secret)), "Token(***)");
    let client = ApiClient::new(&api.base(), Some(&Token::new(secret))).unwrap();
    assert!(!format!("{client:?}").contains(secret));
    let response = client.get("redirect").await.unwrap();
    assert_eq!(response.status, 302);
    assert!(
        client
            .get("http://127.0.0.1:9/steal")
            .await
            .is_err_and(|e| !e.to_string().contains(secret))
    );
    let elsewhere = FakeApi::start("fake_github.py");
    let foreign = client.get(&format!("{}/user", elsewhere.base())).await;
    assert!(foreign.is_err());
    assert!(elsewhere.requests().is_empty(), "{}", elsewhere.requests());
    let offline = ApiClient::new("http://127.0.0.1:9", Some(&Token::new(secret))).unwrap();
    let error = offline.get("user").await.unwrap_err();
    assert!(matches!(error, HttpError::Offline(_)), "{error:?}");
    assert!(!error.to_string().contains(secret));
}

#[test]
fn only_https_servers_or_loopback_are_accepted_and_enterprise_urls_get_the_api_prefix() {
    assert_eq!(
        validate_base("https://git.example.org/").unwrap(),
        "https://git.example.org"
    );
    assert!(validate_base("http://127.0.0.1:8080").is_ok());
    assert!(validate_base("http://localhost").is_ok());
    for bad in [
        "http://git.example.org",
        "ftp://x",
        "git.example.org",
        "https://user:pw@git.example.org",
        "https://",
        "https://a b",
    ] {
        assert!(validate_base(bad).is_err(), "{bad}");
    }
    let enterprise = GithubHost::enterprise("https://ghe.example.org/").unwrap();
    assert_eq!(enterprise.api, "https://ghe.example.org/api/v3");
    assert_eq!(enterprise.web, "https://ghe.example.org");
    assert_eq!(GithubHost::dotcom().api, "https://api.github.com");
    assert!(GithubHost::enterprise("http://ghe.example.org").is_err());
}
