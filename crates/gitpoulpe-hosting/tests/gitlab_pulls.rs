mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{
    GithubError, MemoryStore, PullDraft, PullFilter, PullState, SecretStore, create_pull_request,
    list_pulls, load_pull, prepare_pull,
};

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

fn draft(title: &str, wip: bool) -> PullDraft {
    PullDraft {
        title: title.to_owned(),
        body: "Why".to_owned(),
        draft: wip,
    }
}

fn posts(api: &FakeApi) -> Vec<serde_json::Value> {
    api.requests()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["body"].is_string())
        .collect()
}

const API: (&str, &str) = ("acme/platform", "api");

#[tokio::test]
async fn a_published_branch_is_compared_with_its_target_and_counted_from_the_diffs() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let target = prepare_pull(&store, &conn, API.0, API.1, "feature/x", false, None)
        .await
        .unwrap();
    assert!(target.published && !target.cross_repo());
    assert_eq!(target.base, "main");
    let comparison = target.comparison.unwrap();
    assert_eq!((comparison.ahead, comparison.behind), (2, 1));
    assert_eq!(comparison.files.len(), 2);
    assert_eq!(
        (comparison.files[0].additions, comparison.files[0].deletions),
        (2, 1)
    );
    assert_eq!(comparison.files[1].status, "added");
    assert!(target.existing.is_none());
}

#[tokio::test]
async fn an_unpublished_branch_has_no_comparison_and_cannot_be_proposed() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let target = prepare_pull(&store, &conn, API.0, API.1, "local-only", false, None)
        .await
        .unwrap();
    assert!(!target.published && target.comparison.is_none());
    let refused = create_pull_request(&store, &conn, &target, &draft("T", false)).await;
    assert!(matches!(refused, Err(GithubError::BranchNotPublished(b)) if b == "local-only"));
    assert!(posts(&api).is_empty());
}

#[tokio::test]
async fn a_merge_request_is_created_once_with_the_right_branches() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let target = prepare_pull(&store, &conn, API.0, API.1, "feature/x", false, None)
        .await
        .unwrap();
    let created = create_pull_request(&store, &conn, &target, &draft("  Add x  ", false))
        .await
        .unwrap();
    assert_eq!((created.number, created.state.as_str()), (10, "open"));
    assert_eq!(
        (created.head.as_str(), created.base.as_str()),
        ("feature/x", "main")
    );
    assert!(!created.draft && created.title == "Add x");
    let sent = posts(&api);
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0]["path"]
            .as_str()
            .unwrap()
            .ends_with("/projects/acme%2Fplatform%2Fapi/merge_requests")
    );
    let body: serde_json::Value = serde_json::from_str(sent[0]["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["source_branch"], "feature/x");
    assert_eq!(body["target_branch"], "main");
    assert_eq!(body["title"], "Add x");
    assert_eq!(body["description"], "Why");
    assert!(body.get("target_project_id").is_none());

    let again = create_pull_request(&store, &conn, &target, &draft("Add x", false)).await;
    assert!(matches!(again, Err(GithubError::PullExists(10, _))));
    assert_eq!(posts(&api).len(), 1);
}

#[tokio::test]
async fn an_existing_open_merge_request_is_found_on_preparation_and_blocks_creation() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let target = prepare_pull(&store, &conn, API.0, API.1, "feature/dup", false, None)
        .await
        .unwrap();
    let existing = target.existing.clone().unwrap();
    assert_eq!(existing.number, 5);
    assert!(existing.draft);
    assert_eq!(existing.title, "Work in progress");
    let refused = create_pull_request(&store, &conn, &target, &draft("Again", false)).await;
    assert!(matches!(refused, Err(GithubError::PullExists(5, _))));
    assert!(posts(&api).is_empty());
}

#[tokio::test]
async fn a_draft_merge_request_is_marked_once_in_its_title() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let target = prepare_pull(&store, &conn, API.0, API.1, "feature/x", false, None)
        .await
        .unwrap();
    let created = create_pull_request(&store, &conn, &target, &draft("Draft: Add x", true))
        .await
        .unwrap();
    assert!(created.draft);
    assert_eq!(created.title, "Add x");
    let body: serde_json::Value =
        serde_json::from_str(posts(&api)[0]["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["title"], "Draft: Add x");
}

#[tokio::test]
async fn a_fork_branch_targets_the_parent_project_and_finds_its_own_merge_request() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let found = prepare_pull(&store, &conn, "octo", "api", "feature/y", true, None)
        .await
        .unwrap();
    assert!(found.cross_repo());
    assert_eq!(found.target_repo.full_name(), "acme/platform/api");
    let comparison = found.comparison.unwrap();
    assert_eq!((comparison.ahead, comparison.behind), (3, 4));
    assert_eq!(found.existing.unwrap().number, 7);

    let fresh = prepare_pull(&store, &conn, "octo", "api", "feature/z", true, None)
        .await
        .unwrap();
    assert!(fresh.existing.is_none());
    let created = create_pull_request(&store, &conn, &fresh, &draft("From fork", false))
        .await
        .unwrap();
    assert_eq!(created.head, "feature/z");
    let sent = posts(&api);
    assert!(
        sent[0]["path"]
            .as_str()
            .unwrap()
            .ends_with("/projects/octo%2Fapi/merge_requests")
    );
    let body: serde_json::Value = serde_json::from_str(sent[0]["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["target_project_id"], 100);
}

#[tokio::test]
async fn a_merge_request_from_a_fork_with_the_same_branch_name_is_not_taken_for_a_duplicate() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let target = prepare_pull(&store, &conn, API.0, API.1, "shared/br", false, None)
        .await
        .unwrap();
    assert!(target.existing.is_none());
    let created = create_pull_request(&store, &conn, &target, &draft("Mine", false))
        .await
        .unwrap();
    assert_eq!(created.head, "shared/br");
}

#[tokio::test]
async fn merge_requests_are_listed_by_state_with_merged_ones_counted_as_closed() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let (open, _) = list_pulls(&store, &conn, API.0, API.1, PullFilter::Open)
        .await
        .unwrap();
    let numbers: Vec<u64> = open.iter().map(|p| p.number).collect();
    assert_eq!(numbers, [9, 8, 7, 6, 5]);
    let wip = open.iter().find(|p| p.number == 5).unwrap();
    assert!(wip.draft);
    assert_eq!(wip.title, "Work in progress");
    assert_eq!(wip.author, "octo");
    assert_eq!(wip.head_ref, "feature/dup");
    assert_eq!(wip.base_sha, "b".repeat(40));
    let (closed, _) = list_pulls(&store, &conn, API.0, API.1, PullFilter::Closed)
        .await
        .unwrap();
    let states: Vec<(u64, PullState)> = closed.iter().map(|p| (p.number, p.state)).collect();
    assert_eq!(states, [(4, PullState::Closed), (3, PullState::Merged)]);
    let (all, _) = list_pulls(&store, &conn, API.0, API.1, PullFilter::All)
        .await
        .unwrap();
    assert_eq!(all.len(), 7);
}

#[tokio::test]
async fn every_page_of_diffs_is_read_and_files_without_a_patch_say_so() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let detail = load_pull(&store, &conn, API.0, API.1, 7).await.unwrap();
    let paths: Vec<&str> = detail
        .files
        .iter()
        .map(|f| f.change.path.as_str())
        .collect();
    assert_eq!(
        paths,
        ["src/a.rs", "docs/new.md", "img/logo.png", "big.sql"]
    );
    assert_eq!(
        (
            detail.files[0].change.additions,
            detail.files[0].change.deletions
        ),
        (2, 1)
    );
    assert!(detail.files[0].patch.is_some());
    assert!(detail.files[2].patch.is_none() && detail.files[3].patch.is_none());
    assert!(!detail.files_truncated && !detail.changed_during_load);
    assert_eq!(detail.body, "Body 7");
    assert_eq!(detail.summary.head_sha, "d".repeat(40));
    assert_eq!(detail.summary.base_sha, "b".repeat(40));
    let subjects: Vec<&str> = detail.commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["Second", "First"]);
}

#[tokio::test]
async fn an_older_server_without_the_diffs_endpoint_falls_back_and_flags_the_overflow() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let detail = load_pull(&store, &conn, API.0, API.1, 8).await.unwrap();
    assert_eq!(detail.files.len(), 1);
    assert!(detail.files_truncated);
}

#[tokio::test]
async fn a_merge_request_that_moved_while_loading_is_flagged() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let detail = load_pull(&store, &conn, API.0, API.1, 9).await.unwrap();
    assert!(detail.changed_during_load);
}

#[tokio::test]
async fn unknown_merge_requests_and_malformed_paths_are_refused() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let missing = load_pull(&store, &conn, API.0, API.1, 404).await;
    assert!(matches!(missing, Err(GithubError::NotFound)));
    let before = api.requests().lines().count();
    let bad = load_pull(&store, &conn, "acme/../x", "api", 1).await;
    assert!(matches!(bad, Err(GithubError::NotFound)));
    let bad = list_pulls(&store, &conn, "acme", "a/b", PullFilter::All).await;
    assert!(matches!(bad, Err(GithubError::NotFound)));
    assert_eq!(api.requests().lines().count(), before);
}
