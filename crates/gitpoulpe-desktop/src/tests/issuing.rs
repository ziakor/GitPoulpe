use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use gitpoulpe_core::{Connection, Provider};
use gitpoulpe_hosting::{IssueFilter, MemoryStore, SecretStore};
use rune::{Runtime, TextEdit, prefs::Prefs};

use super::requesting::{Fake, repo as remote_repo};
use super::{click_at, rect_of, runtime, step};
use crate::accounts::Hosting;
use crate::state::State;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}");
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn wait_for(rt: &mut Runtime<State>, what: &str, until: impl Fn(&State) -> bool) {
    for _ in 0..600 {
        step(rt);
        if until(&rt.state) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("never saw {what}");
}

fn github(fake: &Fake) -> Connection {
    Connection {
        id: format!("127.0.0.1:{}/octo", fake.port),
        provider: Provider::Github,
        api: fake.base(),
        web: fake.base(),
        login: "Octo".to_owned(),
        scopes: Vec::new(),
    }
}

fn gitlab(fake: &Fake, login: &str) -> Connection {
    Connection {
        id: format!("127.0.0.1:{}/{}", fake.port, login.to_lowercase()),
        provider: Provider::Gitlab,
        api: format!("{}/api/v4", fake.base()),
        web: fake.base(),
        login: login.to_owned(),
        scopes: Vec::new(),
    }
}

fn app(dir: &Path, connections: &[(&Connection, &str)]) -> (Runtime<State>, Arc<MemoryStore>) {
    let store = Arc::new(MemoryStore::default());
    let mut state = State::new(None, Prefs::memory());
    for (connection, token) in connections {
        store.set(&connection.secret_key(), token).unwrap();
        state.accounts.book.upsert((*connection).clone());
    }
    state.hosting = Hosting::new(store.clone());
    state.engine_git = Some(gitpoulpe_git::Git::new().with_env("GIT_CONFIG_GLOBAL", "/dev/null"));
    state.setup(Arc::new(|| {}));
    state.open_blocking(dir.to_path_buf(), std::time::Duration::from_secs(20));
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    (rt, store)
}

fn numbers(rt: &Runtime<State>) -> Vec<u64> {
    rt.state.issues.items.iter().map(|i| i.number).collect()
}

fn listed(rt: &mut Runtime<State>) {
    wait_for(rt, "the issue list", |s| s.issues.open && !s.issues.loading);
    for _ in 0..10 {
        step(rt);
    }
}

#[test]
fn github_issues_are_listed_filtered_searched_and_read_with_their_context() {
    let fake = Fake::start_script("fake_github.py");
    let conn = github(&fake);
    let dir = remote_repo(&format!("{}/acme/proj.git", fake.base()), "main");
    git(dir.path(), &["branch", "feature/105-crash"]);
    git(dir.path(), &["branch", "feature/1050-unrelated"]);
    let (mut rt, _) = app(dir.path(), &[(&conn, "good")]);
    rt.state.open_issues();
    listed(&mut rt);
    assert_eq!(numbers(&rt), [105, 102, 101]);
    assert!(shows(&rt, "Issues · acme/proj · Octo@"));
    click(&mut rt, "is-all");
    wait_for(&mut rt, "all issues", |s| {
        s.issues.filter == IssueFilter::All && !s.issues.loading && s.issues.items.len() == 4
    });
    assert_eq!(numbers(&rt), [105, 103, 102, 101]);
    rt.state.issues.query = TextEdit::new("dark mode");
    step(&mut rt);
    click(&mut rt, "is-search");
    wait_for(&mut rt, "the search", |s| {
        !s.issues.loading && s.issues.items.len() == 1
    });
    assert_eq!(numbers(&rt), [102]);
    rt.state.issues.query = TextEdit::new("");
    rt.state.issues_set_filter(IssueFilter::Open);
    wait_for(&mut rt, "the open list", |s| {
        !s.issues.loading && s.issues.items.len() == 3
    });
    click(&mut rt, "isk0");
    wait_for(&mut rt, "the detail", |s| s.issues.detail.is_some());
    for _ in 0..10 {
        step(&mut rt);
    }
    assert!(shows(&rt, "#105 Crash on start"));
    assert!(shows(&rt, "dev : Can reproduce.") && shows(&rt, "lead : Fix coming."));
    assert!(shows(&rt, "#12 Fix crash"));
    assert!(shows(&rt, "commit abc1234def"));
    assert!(shows(&rt, "Branches locales liées : feature/105-crash"));
    assert!(!shows(&rt, "feature/1050-unrelated"));
}

#[test]
fn following_a_link_reaches_the_right_pull_request_or_says_the_commit_is_missing() {
    let fake = Fake::start_script("fake_github.py");
    let conn = github(&fake);
    let dir = remote_repo(&format!("{}/acme/proj.git", fake.base()), "main");
    let head = {
        let out = Command::new("git")
            .current_dir(dir.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };
    let (mut rt, _) = app(dir.path(), &[(&conn, "good")]);
    rt.state.open_issues();
    listed(&mut rt);
    click(&mut rt, "isk0");
    wait_for(&mut rt, "the detail", |s| s.issues.detail.is_some());
    for _ in 0..10 {
        step(&mut rt);
    }
    click(&mut rt, "is-commit1");
    step(&mut rt);
    assert!(rt.state.issues.open);
    assert!(shows(&rt, "n'est pas dans ce dépôt local"));
    rt.state.issue_open_commit(&head);
    wait_for(&mut rt, "the commit", |s| !s.issues.open);
    rt.state.open_issues();
    listed(&mut rt);
    click(&mut rt, "isk0");
    wait_for(&mut rt, "the detail again", |s| s.issues.detail.is_some());
    for _ in 0..10 {
        step(&mut rt);
    }
    click(&mut rt, "is-pull0");
    wait_for(&mut rt, "the pull request list", |s| {
        s.pulls.open && s.pulls.list.open && !s.pulls.list.loading && s.pulls.list.notice.is_some()
    });
    assert!(s_notice(&rt).contains("#12"));
}

fn s_notice(rt: &Runtime<State>) -> String {
    rt.state.pulls.list.notice.clone().unwrap_or_default()
}

#[test]
fn a_revoked_account_replaces_the_content_with_an_explicit_refusal() {
    let fake = Fake::start_script("fake_github.py");
    let conn = github(&fake);
    let dir = remote_repo(&format!("{}/acme/proj.git", fake.base()), "main");
    let (mut rt, store) = app(dir.path(), &[(&conn, "good")]);
    rt.state.open_issues();
    listed(&mut rt);
    click(&mut rt, "isk0");
    wait_for(&mut rt, "the detail", |s| s.issues.detail.is_some());
    store.set(&conn.secret_key(), "bad").unwrap();
    rt.state.select_issue(105);
    wait_for(&mut rt, "the refusal", |s| s.issues.detail_error.is_some());
    assert!(rt.state.issues.detail.is_none());
    assert!(
        rt.state
            .issues
            .detail_error
            .as_deref()
            .unwrap()
            .contains("expiré, révoqué ou invalide")
    );
    rt.state.reload_issues();
    wait_for(&mut rt, "the list refusal", |s| s.issues.error.is_some());
    assert!(rt.state.issues.items.is_empty());
    for _ in 0..10 {
        step(&mut rt);
    }
    assert!(shows(&rt, "expiré, révoqué ou invalide"));
}

#[test]
fn a_repository_without_a_matching_account_explains_how_to_connect_one() {
    let fake = Fake::start_script("fake_github.py");
    let dir = remote_repo(&format!("{}/acme/proj.git", fake.base()), "main");
    let (mut rt, _) = app(dir.path(), &[]);
    rt.state.open_issues();
    step(&mut rt);
    assert!(shows(&rt, "Aucun compte connecté ne correspond"));
    assert!(rt.state.issues.items.is_empty());
}

#[test]
fn gitlab_issues_come_from_the_right_instance_with_its_capabilities_and_merge_request_links() {
    let first = Fake::start_script("fake_gitlab.py");
    let second = Fake::start_script("fake_gitlab.py");
    let (a, b) = (gitlab(&first, "Octo"), gitlab(&second, "Other"));
    let dir = remote_repo(&format!("{}/acme/platform/api.git", first.base()), "main");
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "mirror",
            &format!("{}/acme/platform/api.git", second.base()),
        ],
    );
    let (mut rt, _) = app(dir.path(), &[(&a, "glgood"), (&b, "glother")]);
    rt.state.open_issues();
    listed(&mut rt);
    assert_eq!(rt.state.issues.targets.len(), 2);
    let first_is_a = rt.state.issues.targets[0].connection.id == a.id;
    let (ours, theirs) = if first_is_a {
        (vec![11, 10, 8, 7], vec![21])
    } else {
        (vec![21], vec![11, 10, 8, 7])
    };
    assert_eq!(numbers(&rt), ours);
    click(&mut rt, "is-account");
    wait_for(&mut rt, "the other instance", |s| {
        !s.issues.loading && s.issues.items.len() == theirs.len()
    });
    assert_eq!(numbers(&rt), theirs);
    click(&mut rt, "is-account");
    wait_for(&mut rt, "back", |s| {
        !s.issues.loading && s.issues.items.len() == ours.len()
    });
    if !first_is_a {
        click(&mut rt, "is-account");
        wait_for(&mut rt, "the rich instance", |s| {
            !s.issues.loading && s.issues.items.len() == 4
        });
    }
    rt.state.select_issue(11);
    wait_for(&mut rt, "the detail", |s| s.issues.detail.is_some());
    for _ in 0..10 {
        step(&mut rt);
    }
    assert!(shows(&rt, "Serveur : GitLab 16.11.2"));
    assert!(shows(&rt, "#5 Work in progress") && shows(&rt, "#7 Docs"));
    assert!(shows(&rt, "commit 1a2b3c4d5e"));
    assert!(!shows(&rt, "Limite :"));
}

#[test]
fn instance_differences_are_shown_and_the_palette_offers_the_issues() {
    let fake = Fake::start_script("fake_gitlab.py");
    let conn = gitlab(&fake, "Other");
    let dir = remote_repo(&format!("{}/acme/platform/api.git", fake.base()), "main");
    let (mut rt, _) = app(dir.path(), &[(&conn, "glother")]);
    let command = crate::palette::COMMANDS
        .iter()
        .find(|c| c.id == "issues")
        .unwrap();
    assert!((command.available)(&rt.state));
    rt.state.open_issues();
    listed(&mut rt);
    assert_eq!(numbers(&rt), [21]);
    click(&mut rt, "isk0");
    wait_for(&mut rt, "the detail", |s| s.issues.detail.is_some());
    for _ in 0..10 {
        step(&mut rt);
    }
    assert!(shows(
        &rt,
        "Limite : cette instance ne fournit pas « merge requests liées »"
    ));
    assert!(!shows(&rt, "Serveur : GitLab"));
    super::press(&mut rt, rune::Key::Escape);
    assert!(!rt.state.issues.open);
}

#[test]
fn branch_names_are_matched_on_the_whole_number() {
    use crate::issues::mentions_number;
    assert!(mentions_number("feature/105-crash", 105));
    assert!(mentions_number("fix-105", 105));
    assert!(mentions_number("105", 105));
    assert!(mentions_number("issue/#105/crash", 105));
    assert!(!mentions_number("feature/1050-x", 105));
    assert!(!mentions_number("v2105", 105));
    assert!(!mentions_number("feature/10-crash", 105));
}

#[test]
fn a_late_answer_for_a_previously_selected_issue_is_dropped() {
    let fake = Fake::start_script("fake_github.py");
    let conn = github(&fake);
    let dir = remote_repo(&format!("{}/acme/proj.git", fake.base()), "main");
    let (mut rt, _) = app(dir.path(), &[(&conn, "good")]);
    rt.state.open_issues();
    listed(&mut rt);
    rt.state.select_issue(105);
    wait_for(&mut rt, "the first detail", |s| s.issues.detail.is_some());
    let old_request = rt.state.issues.detail_request;
    let old = rt.state.issues.detail.clone().unwrap();
    rt.state.select_issue(102);
    assert!(
        !rt.state
            .apply_issue_loaded(old_request, 105, Ok(Box::new(old.clone())))
    );
    assert!(rt.state.issues.detail.is_none() && rt.state.issues.selected == Some(102));
    let current = rt.state.issues.detail_request;
    assert!(!rt.state.apply_issue_loaded(current, 105, Ok(Box::new(old))));
    wait_for(&mut rt, "the second detail", |s| s.issues.detail.is_some());
    assert_eq!(rt.state.issues.detail.as_ref().unwrap().summary.number, 102);
}
