mod common;

use common::FakeApi;
use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{
    GithubError, IssueFilter, IssueLink, MemoryStore, SecretStore, list_issues, load_issue,
};

fn github(api: &FakeApi) -> Connection {
    Connection {
        id: "gh/octo".to_owned(),
        provider: Provider::Github,
        api: api.base(),
        web: api.base(),
        login: "Octo".to_owned(),
        scopes: Vec::new(),
    }
}

fn gitlab(api: &FakeApi) -> Connection {
    Connection {
        id: "gl/octo".to_owned(),
        provider: Provider::Gitlab,
        api: format!("{}/api/v4", api.base()),
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

fn numbers(items: &[gitpoulpe_hosting::IssueSummary]) -> Vec<u64> {
    items.iter().map(|i| i.number).collect()
}

#[tokio::test]
async fn github_issues_follow_every_page_and_never_include_pull_requests() {
    let api = FakeApi::start("fake_github.py");
    let conn = github(&api);
    let store = stored(&conn, "good");
    let (open, _) = list_issues(&store, &conn, "acme", "proj", IssueFilter::Open, "")
        .await
        .unwrap();
    assert_eq!(numbers(&open), [105, 102, 101]);
    let (all, truncated) = list_issues(&store, &conn, "acme", "proj", IssueFilter::All, "")
        .await
        .unwrap();
    assert_eq!(numbers(&all), [105, 103, 102, 101]);
    assert!(!truncated);
    let (closed, _) = list_issues(&store, &conn, "acme", "proj", IssueFilter::Closed, "")
        .await
        .unwrap();
    assert_eq!(numbers(&closed), [103]);
    let first = &all[0];
    assert_eq!(
        (first.title.as_str(), first.open, first.author.as_str()),
        ("Crash on start", true, "reporter")
    );
    assert_eq!(
        (first.labels.as_slice(), first.comments),
        (&["bug".to_owned()][..], 2)
    );
    assert!(!all[1].open);
}

#[tokio::test]
async fn github_search_stays_inside_the_repository_and_the_issues() {
    let api = FakeApi::start("fake_github.py");
    let conn = github(&api);
    let store = stored(&conn, "good");
    let (hits, _) = list_issues(
        &store,
        &conn,
        "acme",
        "proj",
        IssueFilter::Open,
        "dark mode",
    )
    .await
    .unwrap();
    assert_eq!(numbers(&hits), [102]);
    let (none, _) = list_issues(&store, &conn, "acme", "proj", IssueFilter::All, "zzz")
        .await
        .unwrap();
    assert!(none.is_empty());
    let log = api.requests();
    assert!(log.contains("repo:acme/proj") && log.contains("is:issue"));
}

#[tokio::test]
async fn a_github_issue_is_read_with_its_comments_and_exact_links() {
    let api = FakeApi::start("fake_github.py");
    let conn = github(&api);
    let store = stored(&conn, "good");
    let detail = load_issue(&store, &conn, "acme", "proj", 105)
        .await
        .unwrap();
    assert_eq!(detail.summary.number, 105);
    assert_eq!(detail.body, "It crashes.\nsecond line");
    let authors: Vec<&str> = detail.notes.iter().map(|n| n.author.as_str()).collect();
    assert_eq!(authors, ["dev", "lead"]);
    assert_eq!(
        detail.links,
        [
            IssueLink::Pull {
                number: 12,
                title: "Fix crash".to_owned(),
                open: true
            },
            IssueLink::Commit {
                sha: "abc1234def5678".to_owned()
            },
        ]
    );
    assert!(detail.limits.is_empty() && detail.instance.is_none());
}

#[tokio::test]
async fn rights_are_respected_never_widened_and_always_explicit() {
    let api = FakeApi::start("fake_github.py");
    let conn = github(&api);
    let limited = stored(&conn, "limited");
    assert!(matches!(
        list_issues(&limited, &conn, "acme", "secret", IssueFilter::Open, "").await,
        Err(GithubError::NotFound)
    ));
    let detail = load_issue(&limited, &conn, "acme", "proj", 105)
        .await
        .unwrap();
    assert!(detail.links.is_empty());
    assert!(detail.limits[0].contains("pas lisible avec ce compte"));
    let full = stored(&conn, "good");
    let (secret, _) = list_issues(&full, &conn, "acme", "secret", IssueFilter::Open, "")
        .await
        .unwrap();
    assert_eq!(numbers(&secret), [201]);
    let revoked = stored(&conn, "bad");
    assert!(matches!(
        load_issue(&revoked, &conn, "acme", "proj", 105).await,
        Err(GithubError::Unauthorized)
    ));
    let disabled = list_issues(&full, &conn, "acme", "noissues", IssueFilter::Open, "").await;
    assert!(matches!(disabled, Err(GithubError::Forbidden(m)) if m.contains("désactivées")));
    assert!(matches!(
        load_issue(&full, &conn, "acme", "proj", 999).await,
        Err(GithubError::NotFound)
    ));
    assert!(matches!(
        list_issues(&full, &conn, "../x", "proj", IssueFilter::Open, "").await,
        Err(GithubError::NotFound)
    ));
}

#[tokio::test]
async fn gitlab_issues_page_filter_and_search_within_the_project_namespace() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = gitlab(&api);
    let store = stored(&conn, "glgood");
    let (all, _) = list_issues(&store, &conn, "acme/platform", "api", IssueFilter::All, "")
        .await
        .unwrap();
    assert_eq!(numbers(&all), [11, 10, 9, 8, 7]);
    let (open, _) = list_issues(&store, &conn, "acme/platform", "api", IssueFilter::Open, "")
        .await
        .unwrap();
    assert_eq!(numbers(&open), [11, 10, 8, 7]);
    let (closed, _) = list_issues(
        &store,
        &conn,
        "acme/platform",
        "api",
        IssueFilter::Closed,
        "",
    )
    .await
    .unwrap();
    assert_eq!(numbers(&closed), [9]);
    let (slow, _) = list_issues(
        &store,
        &conn,
        "acme/platform",
        "api",
        IssueFilter::All,
        "slow",
    )
    .await
    .unwrap();
    assert_eq!(numbers(&slow), [10, 7]);
    assert_eq!(
        (all[0].labels.as_slice(), all[0].comments, all[0].open),
        (&["bug".to_owned()][..], 2, true)
    );
    assert!(matches!(
        list_issues(&store, &conn, "acme/../x", "api", IssueFilter::All, "").await,
        Err(GithubError::NotFound)
    ));
}

#[tokio::test]
async fn a_gitlab_issue_links_its_merge_requests_once_and_the_commits_its_notes_mention() {
    let api = FakeApi::start("fake_gitlab.py");
    let conn = gitlab(&api);
    let store = stored(&conn, "glgood");
    let detail = load_issue(&store, &conn, "acme/platform", "api", 11)
        .await
        .unwrap();
    assert_eq!(detail.body, "Login fails.\nsecond line");
    assert_eq!(detail.notes.len(), 1);
    assert_eq!(detail.notes[0].author, "dev");
    assert_eq!(
        detail.links,
        [
            IssueLink::Commit {
                sha: "1a2b3c4d5e6f7a8b".to_owned()
            },
            IssueLink::Pull {
                number: 5,
                title: "Work in progress".to_owned(),
                open: false
            },
            IssueLink::Pull {
                number: 7,
                title: "Docs".to_owned(),
                open: true
            },
        ]
    );
    assert_eq!(detail.instance.as_deref(), Some("GitLab 16.11.2"));
    assert!(detail.limits.is_empty());
}

#[tokio::test]
async fn two_accounts_on_homonymous_projects_see_their_own_issues_and_the_instance_differences() {
    let first = FakeApi::start("fake_gitlab.py");
    let second = FakeApi::start("fake_gitlab.py");
    let (a, b) = (gitlab(&first), gitlab(&second));
    let (store_a, store_b) = (stored(&a, "glgood"), stored(&b, "glother"));
    let (ours, _) = list_issues(&store_a, &a, "acme/platform", "api", IssueFilter::All, "")
        .await
        .unwrap();
    let (theirs, _) = list_issues(&store_b, &b, "acme/platform", "api", IssueFilter::All, "")
        .await
        .unwrap();
    assert_eq!(numbers(&ours), [11, 10, 9, 8, 7]);
    assert_eq!(numbers(&theirs), [21]);
    assert!(!first.requests().contains("glother") && !second.requests().contains("glgood"));
    let detail = load_issue(&store_b, &b, "acme/platform", "api", 21)
        .await
        .unwrap();
    assert!(detail.instance.is_none());
    assert_eq!(detail.limits.len(), 1);
    assert!(detail.limits[0].contains("merge requests liées"));
    assert!(matches!(
        list_issues(&store_b, &b, "octo", "notes", IssueFilter::All, "").await,
        Err(GithubError::NotFound)
    ));
}
