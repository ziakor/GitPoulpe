mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{
    GithubError, MemoryStore, PullFilter, PullState, SecretStore, list_pulls, load_pull,
};

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
async fn open_closed_and_merged_pull_requests_are_told_apart_by_filter() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let (all, truncated) = list_pulls(&store, &conn, "acme", "proj", PullFilter::All)
        .await
        .unwrap();
    assert!(!truncated);
    let state = |n: u64| all.iter().find(|p| p.number == n).unwrap().state;
    assert_eq!(state(1), PullState::Open);
    assert_eq!(state(2), PullState::Closed);
    assert_eq!(state(3), PullState::Merged);
    assert!(all.iter().find(|p| p.number == 4).unwrap().draft);
    let (open, _) = list_pulls(&store, &conn, "acme", "proj", PullFilter::Open)
        .await
        .unwrap();
    assert!(open.iter().all(|p| p.state == PullState::Open));
    let (closed, _) = list_pulls(&store, &conn, "acme", "proj", PullFilter::Closed)
        .await
        .unwrap();
    assert!(closed.iter().any(|p| p.state == PullState::Merged));
    assert!(closed.iter().all(|p| p.state != PullState::Open));
}

#[tokio::test]
async fn a_pull_request_loads_its_revisions_author_commits_and_every_page_of_files() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let detail = load_pull(&store, &conn, "acme", "proj", 1).await.unwrap();
    assert_eq!(detail.summary.author, "author1");
    assert_eq!(detail.summary.head_sha, "sha1");
    assert_eq!(detail.summary.base_sha, "basesha");
    assert_eq!(detail.summary.head_ref, "feature/1");
    assert_eq!(detail.body, "body 1");
    let subjects: Vec<&str> = detail.commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["first", "second"]);
    assert_eq!(detail.commits[0].author, "authorx");
    assert_eq!(detail.commits[1].author, "B");
    assert_eq!(detail.files.len(), 3);
    assert_eq!(detail.files[2].change.path, "f3.rs");
    assert_eq!(
        detail.files[2].patch.as_deref(),
        Some("@@ -1 +1 @@\n-old3\n+new3")
    );
    assert!(!detail.changed_during_load && !detail.files_truncated);
}

#[tokio::test]
async fn a_pull_request_updated_while_loading_is_flagged_and_a_removed_one_is_not_found() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let moving = load_pull(&store, &conn, "acme", "proj", 4).await.unwrap();
    assert!(moving.changed_during_load);
    assert_eq!(
        load_pull(&store, &conn, "acme", "proj", 6)
            .await
            .unwrap_err(),
        GithubError::NotFound
    );
    assert_eq!(
        load_pull(&store, &conn, "../x", "proj", 1)
            .await
            .unwrap_err(),
        GithubError::NotFound
    );
}
