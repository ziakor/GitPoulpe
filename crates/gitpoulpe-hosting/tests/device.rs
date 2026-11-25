mod common;

use std::cell::RefCell;
use std::time::Duration;

use common::FakeApi;
use gitpoulpe_hosting::{
    DevicePoll, GithubError, GithubHost, Grant, MemoryStore, SecretStore, await_device_token,
    check_connection, connect_github, connect_github_grant, disconnect, poll_device_flow,
    renews_itself, start_device_flow,
};

fn host(api: &FakeApi) -> GithubHost {
    GithubHost {
        api: api.base(),
        web: api.base(),
    }
}

#[tokio::test]
async fn the_flow_shows_a_code_waits_slows_down_when_asked_and_ends_with_a_working_connection() {
    let api = FakeApi::start("fake_github.py");
    let host = host(&api);
    let code = start_device_flow(&host, "client-1").await.unwrap();
    assert_eq!(code.user_code, "ABCD-1234");
    assert!(code.verification_uri.ends_with("/login/device"));
    assert_eq!((code.expires_in, code.interval), (900, 5));
    let sleeps = RefCell::new(Vec::new());
    let grant = await_device_token(&host, "client-1", &code, |d: Duration| {
        sleeps.borrow_mut().push(d.as_secs());
        async {}
    })
    .await
    .unwrap();
    assert_eq!(*sleeps.borrow(), [5, 5, 10]);
    let store = MemoryStore::default();
    let (connection, session) = connect_github_grant(&store, &host, grant, "client-1")
        .await
        .unwrap();
    assert_eq!(session.login, "Octo");
    assert!(session.renews_itself && renews_itself(&store, &connection));
    assert_eq!(
        store.get(&connection.secret_key()).unwrap().as_deref(),
        Some("good")
    );
    let log = api.requests();
    assert!(log.contains("\\\"scope\\\":\\\"repo read:org\\\""), "{log}");
    assert!(log.contains("\"accept\": \"application/json\""), "{log}");
    assert!(!log.contains("client_secret"), "{log}");
}

#[tokio::test]
async fn denial_expiry_a_disabled_flow_and_a_wrong_client_id_each_have_their_own_message() {
    let api = FakeApi::start("fake_github.py");
    let host = host(&api);
    assert!(matches!(
        poll_device_flow(&host, "client-1", "dc-deny")
            .await
            .unwrap(),
        DevicePoll::Denied
    ));
    assert!(matches!(
        poll_device_flow(&host, "client-1", "dc-exp").await.unwrap(),
        DevicePoll::Expired
    ));
    assert!(matches!(
        poll_device_flow(&host, "client-1", "dc-off")
            .await
            .unwrap_err(),
        GithubError::Forbidden(_)
    ));
    assert!(matches!(
        start_device_flow(&host, "other").await.unwrap_err(),
        GithubError::Forbidden(_)
    ));
    let code = gitpoulpe_hosting::DeviceCode {
        device_code: "dc-deny".into(),
        user_code: "X".into(),
        verification_uri: api.base(),
        expires_in: 900,
        interval: 5,
    };
    assert_eq!(
        await_device_token(&host, "client-1", &code, |_| async {})
            .await
            .unwrap_err(),
        GithubError::DeviceDenied
    );
    let expired = gitpoulpe_hosting::DeviceCode {
        device_code: "dc-ok".into(),
        expires_in: 5,
        ..code
    };
    assert_eq!(
        await_device_token(&host, "client-1", &expired, |_| async {})
            .await
            .unwrap_err(),
        GithubError::DeviceExpired
    );
}

#[tokio::test]
async fn the_token_obtained_never_shows_in_debug_output() {
    let api = FakeApi::start("fake_github.py");
    let host = host(&api);
    let mut polls = 0;
    let token = loop {
        polls += 1;
        if let DevicePoll::Authorized(t) =
            poll_device_flow(&host, "client-1", "dc-ok").await.unwrap()
        {
            let shown = format!("{t:?}");
            assert!(
                !shown.contains("good") && !shown.contains("ghr-"),
                "{shown}"
            );
            break t;
        }
        assert!(polls < 5);
    };
    assert_eq!(token.access.expose(), "good");
}

async fn brief_grant(host: &GithubHost, code: &str) -> Grant {
    match poll_device_flow(host, "client-1", code).await.unwrap() {
        DevicePoll::Authorized(grant) => grant,
        other => panic!("{other:?}"),
    }
}

fn renewals(api: &FakeApi) -> usize {
    api.requests()
        .lines()
        .filter(|l| l.contains(r#"\"grant_type\":\"refresh_token\""#))
        .count()
}

#[tokio::test]
async fn an_access_token_about_to_expire_is_renewed_once_for_concurrent_requests_and_rotated() {
    let api = FakeApi::start("fake_github.py");
    let host = host(&api);
    let store = MemoryStore::default();
    let grant = brief_grant(&host, "dc-brief").await;
    let (connection, _) = connect_github_grant(&store, &host, grant, "client-1")
        .await
        .unwrap();
    assert_eq!(renewals(&api), 0);
    let (first, second) = tokio::join!(
        check_connection(&store, &connection),
        check_connection(&store, &connection)
    );
    assert_eq!(first.unwrap().login, "Octo");
    assert!(second.unwrap().renews_itself);
    assert_eq!(renewals(&api), 1, "one renewal for both requests");
    let log = api.requests();
    assert!(log.contains(r#"\"refresh_token\":\"ghr-brief\""#), "{log}");
    assert!(!log.contains("client_secret"), "{log}");
    check_connection(&store, &connection).await.unwrap();
    assert_eq!(renewals(&api), 1, "the renewed token lasts eight hours");
    assert!(renews_itself(&store, &connection));
}

#[tokio::test]
async fn a_refused_renewal_asks_to_reconnect_and_a_pasted_token_or_a_disconnect_clears_it() {
    let api = FakeApi::start("fake_github.py");
    let host = host(&api);
    let store = MemoryStore::default();
    let grant = brief_grant(&host, "dc-dead").await;
    let (connection, _) = connect_github_grant(&store, &host, grant, "client-1")
        .await
        .unwrap();
    assert_eq!(
        check_connection(&store, &connection).await.unwrap_err(),
        GithubError::SessionExpired
    );
    assert!(!renews_itself(&store, &connection));
    assert!(
        GithubError::SessionExpired
            .to_string()
            .contains("reconnectez"),
        "the message says what to do"
    );

    let grant = brief_grant(&host, "dc-brief").await;
    let (connection, _) = connect_github_grant(&store, &host, grant, "client-1")
        .await
        .unwrap();
    assert!(renews_itself(&store, &connection));
    connect_github(&store, &host, "good").await.unwrap();
    assert!(
        !renews_itself(&store, &connection),
        "a pasted token does not keep an old renewal"
    );
    let grant = brief_grant(&host, "dc-brief").await;
    connect_github_grant(&store, &host, grant, "client-1")
        .await
        .unwrap();
    disconnect(&store, &connection).unwrap();
    assert!(store.keys().is_empty(), "{:?}", store.keys());
}
