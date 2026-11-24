use std::path::Path;
use std::process::Command;

use gitpoulpe_core::Tracking;
use rune::Runtime;

use super::{click_at, rect_of, runtime, step};
use crate::state::State;
use crate::sync::Link;

fn git(dir: &Path, args: &[&str]) -> String {
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
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref() == Some(needle))
}

fn settle(rt: &mut Runtime<State>, until: impl Fn(&State) -> bool) {
    for _ in 0..400 {
        step(rt);
        if until(&rt.state) {
            for _ in 0..8 {
                step(rt);
            }
            return;
        }
    }
    panic!("the screen never settled: notice={:?}", rt.state.notice);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn reread(rt: &mut Runtime<State>, until: impl Fn(&State) -> bool) {
    rt.update(|s, _| s.refresh());
    settle(rt, until);
}

struct Env {
    repo: tempfile::TempDir,
    bare: tempfile::TempDir,
}

fn env() -> Env {
    let bare = tempfile::tempdir().unwrap();
    git(bare.path(), &["init", "-q", "--bare", "-b", "main"]);
    let seed = tempfile::tempdir().unwrap();
    git(seed.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(seed.path().join("a.txt"), "one\n").unwrap();
    git(seed.path(), &["add", "-A"]);
    git(seed.path(), &["commit", "-q", "-m", "first"]);
    git(
        seed.path(),
        &["push", "-q", &bare.path().display().to_string(), "main"],
    );
    let repo = tempfile::tempdir().unwrap();
    git(
        repo.path(),
        &["clone", "-q", &bare.path().display().to_string(), "."],
    );
    git(
        repo.path(),
        &[
            "remote",
            "set-url",
            "origin",
            &format!("file://{}", bare.path().display()),
        ],
    );
    Env { repo, bare }
}

fn push_from_elsewhere(e: &Env) {
    let clone = tempfile::tempdir().unwrap();
    git(
        clone.path(),
        &["clone", "-q", &e.bare.path().display().to_string(), "."],
    );
    std::fs::write(clone.path().join("b.txt"), "two\n").unwrap();
    git(clone.path(), &["add", "-A"]);
    git(clone.path(), &["commit", "-q", "-m", "remote"]);
    git(clone.path(), &["push", "-q", "origin", "main"]);
}

fn tracking(s: &State) -> Option<Tracking> {
    match s.link() {
        Some(Link::Tracked(_, tracking)) => Some(tracking),
        _ => None,
    }
}

#[test]
fn fetching_from_the_top_bar_reports_what_arrived_and_that_the_branch_must_be_synchronised() {
    let e = env();
    let mut rt = runtime(super::opened(e.repo.path()));
    assert!(shows(&rt, "à jour"));
    assert!(shows(&rt, "avec origin"));
    push_from_elsewhere(&e);
    let head = git(e.repo.path(), &["rev-parse", "HEAD"]);
    click(&mut rt, "sync-fetch");
    settle(&mut rt, |s| {
        tracking(s) == Some(Tracking::Differs) && s.notice.is_some()
    });
    assert_eq!(
        rt.state.notice.as_deref(),
        Some("origin récupéré : 1 référence mise à jour.")
    );
    assert!(shows(&rt, "à synchroniser"));
    assert_eq!(git(e.repo.path(), &["rev-parse", "HEAD"]), head);
}

#[test]
fn a_failed_fetch_is_explained_in_the_status_line() {
    let e = env();
    let mut rt = runtime(super::opened(e.repo.path()));
    git(
        e.repo.path(),
        &["remote", "set-url", "origin", "file:///nowhere/gitpoulpe"],
    );
    click(&mut rt, "sync-fetch");
    settle(&mut rt, |s| {
        s.notice.is_some() && s.sync_progress().is_none()
    });
    let notice = rt.state.notice.clone().unwrap();
    assert!(notice.starts_with("Récupération impossible : "), "{notice}");
    assert!(shows(&rt, "Récupérer"));
}

#[test]
fn integrate_and_publish_open_the_current_branch_with_their_preview() {
    let e = env();
    push_from_elsewhere(&e);
    git(e.repo.path(), &["fetch", "-q"]);
    let mut rt = runtime(super::opened(e.repo.path()));
    rt.update(|s, _| s.open_history());
    settle(&mut rt, |s| s.history_open);
    click(&mut rt, "sync-pull");
    settle(&mut rt, |s| {
        s.pulling
            .state
            .as_ref()
            .is_some_and(|p| p.preview.is_some())
            && !s.branches.loading
    });
    assert!(!rt.state.history_open);
    assert!(rt.state.branches.open);
    assert_eq!(rt.state.branches.selected.as_deref(), Some("main"));
    assert!(rect_of(&rt, "pull-panel").is_some());
    click(&mut rt, "sync-publish");
    settle(&mut rt, |s| s.publishing.state.is_some());
    assert!(rt.state.pulling.state.is_none());
    assert_eq!(
        rt.state
            .publishing
            .state
            .as_ref()
            .map(|p| p.branch.as_str()),
        Some("main")
    );
    assert!(rect_of(&rt, "publish-panel").is_some());
}

#[test]
fn the_offered_actions_follow_what_the_branch_tracks() {
    let e = env();
    let mut rt = runtime(super::opened(e.repo.path()));
    for key in ["sync-fetch", "sync-pull", "sync-publish"] {
        assert!(rect_of(&rt, key).is_some(), "{key} is missing");
    }
    git(
        e.repo.path(),
        &["update-ref", "-d", "refs/remotes/origin/main"],
    );
    reread(&mut rt, |s| matches!(s.link(), Some(Link::Missing(_))));
    assert!(shows(&rt, "branche suivie introuvable"));
    assert!(rect_of(&rt, "sync-fetch").is_some());
    assert!(rect_of(&rt, "sync-pull").is_none());
    git(e.repo.path(), &["checkout", "-q", "-b", "local"]);
    reread(&mut rt, |s| s.link() == Some(Link::Local));
    assert!(shows(&rt, "pas de branche distante"));
    assert!(rect_of(&rt, "sync-fetch").is_none());
    assert!(rect_of(&rt, "sync-publish").is_some());
    git(e.repo.path(), &["checkout", "-q", "--detach"]);
    reread(&mut rt, |s| s.link() == Some(Link::Detached));
    assert!(shows(&rt, "HEAD détachée"));
    assert!(rect_of(&rt, "sync").is_none());
}
