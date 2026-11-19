mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{
    GithubError, MemoryStore, PullDraft, SecretStore, create_pull_request, prepare_pull,
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

fn draft(title: &str, draft: bool) -> PullDraft {
    PullDraft {
        title: title.to_owned(),
        body: "body".to_owned(),
        draft,
    }
}

fn creations(api: &FakeApi) -> usize {
    api.requests().matches("\"path\": \"/api/v3/repos").count()
        + api.requests().matches("pulls\", \"auth\"").count()
}

fn posts(api: &FakeApi) -> usize {
    api.requests().matches("\"body\": \"{\\\"base\\\"").count()
}

#[tokio::test]
async fn an_intra_repository_pull_shows_branches_and_diff_then_is_created_with_its_draft_state() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let prepared = prepare_pull(&store, &conn, "octo", "proj", "feature/x", false, None)
        .await
        .unwrap();
    assert!(prepared.published && !prepared.cross_repo());
    assert_eq!(prepared.base, "main");
    let comparison = prepared.comparison.clone().unwrap();
    assert_eq!((comparison.ahead, comparison.behind), (2, 1));
    assert_eq!(comparison.files.len(), 2);
    assert_eq!(comparison.files[0].path, "src/a.rs");
    assert!(prepared.existing.is_none());
    let created = create_pull_request(&store, &conn, &prepared, &draft(" Add a ", true))
        .await
        .unwrap();
    assert_eq!((created.number, created.draft), (12, true));
    assert_eq!(created.title, "Add a");
    assert_eq!(
        (created.head.as_str(), created.base.as_str()),
        ("feature/x", "main")
    );
    assert_eq!(posts(&api), 1);
}

#[tokio::test]
async fn a_pull_from_a_fork_targets_the_parent_with_an_owner_qualified_head() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let prepared = prepare_pull(&store, &conn, "octo", "forked", "feature/x", true, None)
        .await
        .unwrap();
    assert!(prepared.cross_repo());
    assert_eq!(prepared.target_repo.full_name(), "acme/proj");
    assert!(prepared.comparison.is_some());
    let created = create_pull_request(&store, &conn, &prepared, &draft("T", false))
        .await
        .unwrap();
    assert_eq!(created.head, "octo:feature/x");
    let same = prepare_pull(&store, &conn, "octo", "forked", "feature/x", false, None)
        .await
        .unwrap();
    assert!(!same.cross_repo());
}

#[tokio::test]
async fn an_unpublished_branch_an_existing_pull_and_a_refusal_each_give_an_explicit_next_step_without_duplicates()
 {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    let local = prepare_pull(&store, &conn, "octo", "proj", "local-only", false, None)
        .await
        .unwrap();
    assert!(!local.published && local.comparison.is_none());
    assert_eq!(
        create_pull_request(&store, &conn, &local, &draft("T", false))
            .await
            .unwrap_err(),
        GithubError::BranchNotPublished("local-only".to_owned())
    );
    let dup = prepare_pull(&store, &conn, "octo", "proj", "feature/dup", false, None)
        .await
        .unwrap();
    assert_eq!(dup.existing.as_ref().map(|p| p.number), Some(7));
    let error = create_pull_request(&store, &conn, &dup, &draft("T", false))
        .await
        .unwrap_err();
    assert!(matches!(error, GithubError::PullExists(7, _)), "{error:?}");
    assert_eq!(posts(&api), 0);
    let bad = prepare_pull(&store, &conn, "octo", "proj", "feature/bad", false, None)
        .await
        .unwrap();
    let error = create_pull_request(&store, &conn, &bad, &draft("T", false))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, GithubError::Validation(m) if m.contains("No commits between")),
        "{error:?}"
    );
    assert!(matches!(
        create_pull_request(&store, &conn, &bad, &draft("  ", false))
            .await
            .unwrap_err(),
        GithubError::Validation(_)
    ));
    assert_eq!(posts(&api), 1);
    let _ = creations(&api);
}

#[tokio::test]
async fn bad_branch_names_never_reach_the_network() {
    let api = FakeApi::start("fake_github.py");
    let conn = connection(&api);
    let store = stored(&conn);
    for branch in ["", "../x", "a b", "a?x", "/lead", "a//b"] {
        assert_eq!(
            prepare_pull(&store, &conn, "octo", "proj", branch, false, None)
                .await
                .unwrap_err(),
            GithubError::NotFound,
            "{branch:?}"
        );
    }
    assert_eq!(
        prepare_pull(&store, &conn, "octo", "proj", "wip", false, Some("a b"))
            .await
            .unwrap_err(),
        GithubError::NotFound
    );
    assert!(api.requests().is_empty());
}
