mod common;

use std::cell::RefCell;
use std::time::Duration;

use common::FakeApi;
use gitpoulpe_hosting::{
    DevicePoll, GithubError, GithubHost, MemoryStore, SecretStore, await_device_token,
    connect_github, poll_device_flow, start_device_flow,
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
    let token = await_device_token(&host, "client-1", &code, |d: Duration| {
        sleeps.borrow_mut().push(d.as_secs());
        async {}
    })
    .await
    .unwrap();
    assert_eq!(*sleeps.borrow(), [5, 5, 10]);
    let store = MemoryStore::default();
    let (connection, session) = connect_github(&store, &host, token.expose()).await.unwrap();
    assert_eq!(session.login, "Octo");
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
            assert!(!format!("{t:?}").contains("good"));
            break t;
        }
        assert!(polls < 5);
    };
    assert_eq!(token.expose(), "good");
}
