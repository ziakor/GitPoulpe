mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{
    CheckState, GithubError, MemoryStore, MergeMethod, MergeRequest, PullState, SecretStore,
    is_merged, merge_pull, prepare_merge,
};

const API: (&str, &str) = ("acme/platform", "api");

fn sha(c: char) -> String {
    c.to_string().repeat(40)
}

fn connection(api: &FakeApi) -> Connection {
    Connection {
        id: "gl/octo".to_owned(),
        provider: Provider::Gitlab,
        api: format!("{}/api/v4", api.base()),
        web: api.base(),
        login: "Octo".to_owned(),
        scopes: Vec::new(),
    }
}

fn stored(connection: &Connection) -> MemoryStore {
    let store = MemoryStore::default();
    store.set(&connection.secret_key(), "glgood").unwrap();
    store
}

fn puts(api: &FakeApi) -> Vec<serde_json::Value> {
    api.requests()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["method"] == "PUT")
        .map(|v| serde_json::from_str(v["body"].as_str().unwrap()).unwrap())
        .collect()
}

async fn blockers(repo: (&str, &str), number: u64) -> Vec<String> {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    prepare_merge(&store, &conn, repo.0, repo.1, number)
        .await
        .unwrap()
        .blockers
}

#[tokio::test]
async fn a_clean_merge_request_offers_the_project_methods_and_no_blocker() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let ready = prepare_merge(&store, &conn, API.0, API.1, 20)
        .await
        .unwrap();
    assert!(ready.blockers.is_empty(), "{:?}", ready.blockers);
    assert_eq!(ready.state, PullState::Open);
    assert_eq!(ready.head_sha, sha('2'));
    assert_eq!(ready.methods, [MergeMethod::Merge, MergeMethod::Squash]);
    assert_eq!(ready.required_checks.len(), 1);
    assert_eq!(ready.required_checks[0].state, Some(CheckState::Success));
    assert!(ready.can_attempt() && !ready.protection_unreadable);
}

#[tokio::test]
async fn merge_methods_follow_the_project_settings() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let fast_forward = prepare_merge(&store, &conn, "acme/platform", "dev", 40)
        .await
        .unwrap();
    assert_eq!(
        fast_forward.methods,
        [MergeMethod::Rebase, MergeMethod::Squash]
    );
    let always = prepare_merge(&store, &conn, "octo", "api", 20)
        .await
        .unwrap();
    assert_eq!(always.methods, [MergeMethod::Squash]);
    let never = prepare_merge(&store, &conn, "octo", "notes", 20)
        .await
        .unwrap();
    assert_eq!(never.methods, [MergeMethod::Merge]);
    assert!(never.protection_unreadable);
}

#[tokio::test]
async fn every_gitlab_blocker_is_listed_in_plain_words() {
    let has = |list: &[String], needle: &str| list.iter().any(|b| b.contains(needle));
    assert!(has(
        &blockers(API, 21).await,
        "1 approbation sur 2 requises"
    ));
    assert!(has(&blockers(API, 22).await, "conflits"));
    assert!(has(&blockers(API, 23).await, "« Pipeline » est en attente"));
    assert!(has(&blockers(API, 31).await, "« Pipeline » n'a pas réussi"));
    assert!(has(&blockers(API, 32).await, "« Pipeline » est absente"));
    assert!(has(&blockers(API, 24).await, "rebasée"));
    assert!(has(
        &blockers(API, 33).await,
        "changements ont été demandés"
    ));
    assert!(has(&blockers(API, 41).await, "calcule encore"));
    let protected = blockers(("acme/platform", "dev"), 40).await;
    assert!(has(&protected, "est protégée"), "{protected:?}");
    assert!(blockers(API, 20).await.is_empty());
}

#[tokio::test]
async fn merging_names_the_revision_and_reads_the_remote_state_back() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let request = MergeRequest {
        method: MergeMethod::Merge,
        expected_head: sha('2'),
        message: " Merge it ".to_owned(),
    };
    let done = merge_pull(&store, &conn, API.0, API.1, 20, &request)
        .await
        .unwrap();
    assert_eq!((done.sha, done.verified_merged), (sha('m'), true));
    let sent = puts(&api);
    assert_eq!(sent[0]["sha"], sha('2'));
    assert_eq!(sent[0]["squash"], false);
    assert_eq!(sent[0]["merge_commit_message"], "Merge it");
    assert!(is_merged(&store, &conn, API.0, API.1, 20).await.unwrap());
}

#[tokio::test]
async fn a_squash_merge_sends_its_own_message() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let request = MergeRequest {
        method: MergeMethod::Squash,
        expected_head: sha('f'),
        message: "One commit".to_owned(),
    };
    merge_pull(&store, &conn, "acme/platform", "dev", 40, &request)
        .await
        .unwrap();
    let sent = puts(&api);
    assert_eq!(sent[0]["squash"], true);
    assert_eq!(sent[0]["squash_commit_message"], "One commit");
    assert!(sent[0].get("merge_commit_message").is_none());
}

#[tokio::test]
async fn refusals_and_a_moved_revision_never_look_like_a_merge() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let request = |head: char| MergeRequest {
        method: MergeMethod::Merge,
        expected_head: sha(head),
        message: String::new(),
    };
    let blocked = merge_pull(&store, &conn, API.0, API.1, 27, &request('1')).await;
    assert!(matches!(blocked, Err(GithubError::MergeRefused(_))));
    let forbidden = merge_pull(&store, &conn, API.0, API.1, 28, &request('a')).await;
    assert!(matches!(forbidden, Err(GithubError::MergeRefused(m)) if m.contains("droit")));
    let before = puts(&api).len();
    let moved = merge_pull(&store, &conn, API.0, API.1, 20, &request('9')).await;
    assert!(matches!(moved, Err(GithubError::AnchorStale { .. })));
    assert_eq!(puts(&api).len(), before);
    assert!(!is_merged(&store, &conn, API.0, API.1, 20).await.unwrap());
}
