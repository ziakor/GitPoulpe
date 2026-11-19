use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use gitpoulpe_core::{Connection, Notice, NoticeBook, NoticeKind, Provider};
use gitpoulpe_hosting::{MemoryStore, RawNotice, SecretStore};
use rune::{Runtime, prefs::Prefs};

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

fn plain_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    dir
}

struct Setup {
    github: Fake,
    gitlab: Fake,
    store: Arc<MemoryStore>,
    gh: Connection,
    gl: Connection,
}

fn setup() -> Setup {
    let github = Fake::start_script("fake_github.py");
    let gitlab = Fake::start_script("fake_gitlab.py");
    let store = Arc::new(MemoryStore::default());
    let gh = Connection {
        id: format!("127.0.0.1:{}/octo", github.port),
        provider: Provider::Github,
        api: github.base(),
        web: github.base(),
        login: "Octo".to_owned(),
        scopes: Vec::new(),
    };
    let gl = Connection {
        id: format!("127.0.0.1:{}/octo", gitlab.port),
        provider: Provider::Gitlab,
        api: format!("{}/api/v4", gitlab.base()),
        web: gitlab.base(),
        login: "Octo".to_owned(),
        scopes: Vec::new(),
    };
    store.set(&gh.secret_key(), "good").unwrap();
    store.set(&gl.secret_key(), "glgood").unwrap();
    Setup {
        github,
        gitlab,
        store,
        gh,
        gl,
    }
}

fn app(dir: &Path, setup: &Setup) -> Runtime<State> {
    let mut state = State::new(None, Prefs::memory());
    state.accounts.book.upsert(setup.gh.clone());
    state.accounts.book.upsert(setup.gl.clone());
    state.hosting = Hosting::new(setup.store.clone());
    state.engine_git = Some(gitpoulpe_git::Git::new().with_env("GIT_CONFIG_GLOBAL", "/dev/null"));
    state.setup(Arc::new(|| {}));
    state.open_blocking(dir.to_path_buf(), std::time::Duration::from_secs(20));
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    rt
}

fn refreshed(rt: &mut Runtime<State>) {
    rt.state.open_notices();
    step(rt);
    click(rt, "nt-refresh");
    wait_for(rt, "the inbox", |s| {
        s.notices.fetching == 0 && s.notices.request > 0
    });
    for _ in 0..10 {
        step(rt);
    }
}

#[test]
fn both_providers_feed_the_inbox_and_each_notice_names_its_own_account() {
    let setup = setup();
    let dir = plain_repo();
    let mut rt = app(dir.path(), &setup);
    refreshed(&mut rt);
    assert_eq!(rt.state.notices.book.items.len(), 7);
    let kinds = |conn: &str| -> Vec<NoticeKind> {
        rt.state
            .notices
            .book
            .items
            .iter()
            .filter(|n| n.connection.as_deref() == Some(conn))
            .map(|n| n.kind)
            .collect()
    };
    assert_eq!(kinds(&setup.gh.id).len(), 3);
    assert_eq!(kinds(&setup.gl.id).len(), 4);
    assert!(shows(&rt, "PR 1") && shows(&rt, "Work in progress"));
    assert!(shows(&rt, &format!("Octo@127.0.0.1:{}", setup.github.port)));
    assert!(shows(&rt, &format!("Octo@127.0.0.1:{}", setup.gitlab.port)));
    assert!(shows(&rt, "7 non lue(s)"));
}

#[test]
fn repeated_reads_and_bursts_are_grouped_not_multiplied() {
    let setup = setup();
    let dir = plain_repo();
    let mut rt = app(dir.path(), &setup);
    refreshed(&mut rt);
    click(&mut rt, "nt-refresh");
    wait_for(&mut rt, "the second read", |s| s.notices.fetching == 0);
    assert_eq!(rt.state.notices.book.items.len(), 7);
    rt.state.notices_mark_all_read();
    assert_eq!(rt.state.notices_unread(), 0);
    let request = rt.state.notices.request;
    for n in 0..5 {
        let burst = vec![RawNotice {
            id: format!("burst{n}"),
            kind: NoticeKind::Mention,
            title: "Question".to_owned(),
            repo: Some("acme/proj".to_owned()),
            number: Some(12),
            at: format!("2026-11-19T10:0{n}:00Z"),
        }];
        rt.state.notices.fetching = 1;
        assert!(
            rt.state
                .apply_notices_fetched(request, setup.gh.id.clone(), Ok(burst))
        );
    }
    assert_eq!(rt.state.notices.book.items.len(), 7);
    let grouped = rt
        .state
        .notices
        .book
        .items
        .iter()
        .find(|n| n.number == Some(12))
        .unwrap();
    assert_eq!(grouped.count, 6);
    assert!(!grouped.read);
    for _ in 0..10 {
        step(&mut rt);
    }
    assert!(shows(&rt, "(×6)"));
    assert_eq!(rt.state.notices_unread(), 1);
}

#[test]
fn preferences_decide_what_is_read_shown_and_remembered() {
    let setup = setup();
    let dir = plain_repo();
    let mut rt = app(dir.path(), &setup);
    rt.state.open_notices();
    step(&mut rt);
    click(&mut rt, "nt-kind-mention");
    click(&mut rt, "nt-account1");
    step(&mut rt);
    rt.state.notices_refresh();
    assert_eq!(
        rt.state.notices.fetching, 1,
        "a muted account is not even asked"
    );
    wait_for(&mut rt, "the filtered read", |s| {
        s.notices.fetching == 0 && s.notices.request > 0
    });
    let items = &rt.state.notices.book.items;
    assert!(items.iter().all(|n| n.kind != NoticeKind::Mention));
    assert!(
        items
            .iter()
            .all(|n| n.connection.as_deref() != Some(setup.gl.id.as_str()))
    );
    assert_eq!(items.len(), 2);
    let saved: NoticeBook = rt.state.prefs.get(crate::notices::NOTICES_KEY).unwrap();
    assert!(saved.prefs.muted_kinds.contains(&NoticeKind::Mention));
    let mut prefs = Prefs::memory();
    prefs.set(crate::notices::NOTICES_KEY, &saved).unwrap();
    let again = State::new(None, prefs);
    assert!(
        again
            .notices
            .book
            .prefs
            .muted_connections
            .contains(&setup.gl.id)
    );
    click(&mut rt, "nt-enabled");
    step(&mut rt);
    let before = rt.state.notices.request;
    click(&mut rt, "nt-refresh");
    step(&mut rt);
    assert!(shows(&rt, "désactivées"));
    assert_eq!(rt.state.notices.fetching, 0);
    assert!(rt.state.notices.request > before);
    assert_eq!(rt.state.notices_unread(), 0);
}

#[test]
fn opening_a_notice_joins_the_matching_local_repository_and_the_pull_request() {
    let setup = setup();
    let other = plain_repo();
    let matching = remote_repo(&format!("{}/acme/proj.git", setup.github.base()), "main");
    let mut rt = app(other.path(), &setup);
    rt.state
        .favorites_add_path(matching.path().to_path_buf(), None);
    refreshed(&mut rt);
    let index = rt
        .state
        .notices
        .book
        .visible()
        .position(|n| n.title == "PR 1")
        .unwrap();
    click(&mut rt, &format!("nt-open{index}"));
    wait_for(&mut rt, "the pull request", |s| {
        s.pulls.list.open && s.pulls.list.selected == Some(1) && s.pulls.list.detail.is_some()
    });
    assert!(
        rt.state
            .repo
            .as_ref()
            .unwrap()
            .path
            .ends_with(matching.path().file_name().unwrap())
    );
    assert!(
        rt.state
            .notices
            .book
            .items
            .iter()
            .find(|n| n.title == "PR 1")
            .unwrap()
            .read
    );
}

#[test]
fn a_notice_for_a_repository_that_is_not_cloned_prefills_the_clone_screen() {
    let setup = setup();
    let dir = plain_repo();
    let mut rt = app(dir.path(), &setup);
    refreshed(&mut rt);
    let index = rt
        .state
        .notices
        .book
        .visible()
        .position(|n| n.title == "PR 1")
        .unwrap();
    click(&mut rt, &format!("nt-open{index}"));
    step(&mut rt);
    assert!(rt.state.clone.open);
    assert_eq!(
        rt.state.clone.url.text,
        format!("{}/acme/proj.git", setup.github.base())
    );
}

#[test]
fn a_vanished_pull_request_says_so_instead_of_showing_something_else() {
    let setup = setup();
    let other = plain_repo();
    let matching = remote_repo(&format!("{}/acme/proj.git", setup.github.base()), "main");
    let mut rt = app(other.path(), &setup);
    rt.state
        .favorites_add_path(matching.path().to_path_buf(), None);
    rt.state.notices.book.push(Notice {
        id: "stale".to_owned(),
        connection: Some(setup.gh.id.clone()),
        repo_path: None,
        kind: NoticeKind::Review,
        title: "Gone".to_owned(),
        repo: Some("acme/proj".to_owned()),
        number: Some(999),
        at: "t".to_owned(),
        count: 1,
        read: false,
    });
    rt.state.open_notices();
    step(&mut rt);
    click(&mut rt, "nt-open0");
    wait_for(&mut rt, "the explanation", |s| {
        s.pulls.list.open && !s.pulls.list.loading && s.pulls.list.notice.is_some()
    });
    assert!(rt.state.pulls.list.selected.is_none());
    assert!(
        rt.state
            .pulls
            .list
            .notice
            .as_deref()
            .unwrap()
            .contains("#999")
    );
}

#[test]
fn a_revoked_or_disconnected_account_never_opens_content_from_another_connection() {
    let setup = setup();
    let dir = plain_repo();
    let mut rt = app(dir.path(), &setup);
    refreshed(&mut rt);
    setup.store.set(&setup.gh.secret_key(), "bad").unwrap();
    click(&mut rt, "nt-refresh");
    wait_for(&mut rt, "the refusal", |s| {
        s.notices.fetching == 0 && s.notices.errors.contains_key(&setup.gh.id)
    });
    for _ in 0..10 {
        step(&mut rt);
    }
    assert!(shows(&rt, "Lecture impossible pour Octo@"));
    let index = rt
        .state
        .notices
        .book
        .visible()
        .position(|n| n.title == "PR 1")
        .unwrap();
    click(&mut rt, &format!("nt-open{index}"));
    step(&mut rt);
    assert!(shows(
        &rt,
        "reconnectez le compte avant d'ouvrir cette notification"
    ));
    assert!(rt.state.notices.open && !rt.state.clone.open && !rt.state.pulls.open);
    rt.state.accounts.book.remove(&setup.gh.id);
    let stale = rt.state.notices.request;
    assert!(
        !rt.state
            .apply_notices_fetched(stale, setup.gh.id.clone(), Ok(Vec::new()))
    );
    rt.state.notices_open("never-existed");
    let id = rt
        .state
        .notices
        .book
        .items
        .iter()
        .find(|n| n.connection.as_deref() == Some(setup.gh.id.as_str()))
        .unwrap()
        .id
        .clone();
    rt.state.notices_open(&id);
    step(&mut rt);
    assert!(shows(&rt, "n'est plus connecté"));
    assert!(shows(&rt, "compte déconnecté"));
}

#[test]
fn a_merge_in_progress_raises_a_local_notice_that_leads_back_to_its_conflicts() {
    let setup = setup();
    let a = plain_repo();
    let b = tempfile::tempdir().unwrap();
    let p = b.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "base\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "base"]);
    git(p, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(p.join("a.txt"), "feature\n").unwrap();
    git(p, &["commit", "-qam", "feature"]);
    git(p, &["checkout", "-q", "main"]);
    std::fs::write(p.join("a.txt"), "main\n").unwrap();
    git(p, &["commit", "-qam", "main"]);
    let merge = Command::new("git")
        .current_dir(p)
        .args(["merge", "feature"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(!merge.status.success() && p.join(".git/MERGE_HEAD").exists());

    let mut rt = app(b.path(), &setup);
    wait_for(&mut rt, "the local notice", |s| {
        s.notices
            .book
            .items
            .iter()
            .any(|n| n.kind == NoticeKind::Conflicts)
    });
    let notice = rt.state.notices.book.items[0].clone();
    assert!(notice.title.contains("fusion en cours") && notice.connection.is_none());
    rt.state.open_path(a.path().to_path_buf());
    wait_for(&mut rt, "repository A", |s| {
        s.repo
            .as_ref()
            .is_some_and(|r| r.path.ends_with(a.path().file_name().unwrap()))
    });
    rt.state.open_notices();
    step(&mut rt);
    click(&mut rt, "nt-open0");
    wait_for(&mut rt, "the conflicts of B", |s| {
        s.conflicts.open
            && s.repo
                .as_ref()
                .is_some_and(|r| r.path.ends_with(b.path().file_name().unwrap()))
    });
}

#[test]
fn the_palette_offers_the_inbox() {
    let command = crate::palette::COMMANDS
        .iter()
        .find(|c| c.id == "notices")
        .unwrap();
    let mut rt = runtime(State::new(None, Prefs::memory()));
    assert!((command.available)(&rt.state));
    rt.state.open_notices();
    step(&mut rt);
    assert!(shows(&rt, "Aucune notification."));
}
