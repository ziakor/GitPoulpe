use std::path::Path;
use std::process::Command;

use rune::Runtime;

use super::{click_at, rect_of, runtime, step};
use crate::state::State;

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
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn settle(rt: &mut Runtime<State>, until: impl Fn(&State) -> bool) {
    for _ in 0..100 {
        step(rt);
        if until(&rt.state) {
            break;
        }
    }
    for _ in 0..12 {
        step(rt);
    }
    for _ in 0..100 {
        if until(&rt.state) {
            return;
        }
        step(rt);
    }
    panic!("the screen never settled: notice={:?}", rt.state.notice);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    if key.starts_with("publish-") || key == "branch-publish" {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("branch-panel", target));
        for _ in 0..8 {
            step(rt);
        }
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

struct Env {
    repo: tempfile::TempDir,
    bare: tempfile::TempDir,
}

fn env() -> Env {
    let bare = tempfile::tempdir().unwrap();
    git(bare.path(), &["init", "-q", "--bare", "-b", "main"]);
    let repo = tempfile::tempdir().unwrap();
    let p = repo.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "first"]);
    git(
        p,
        &[
            "remote",
            "add",
            "origin",
            &format!("file://{}", bare.path().display()),
        ],
    );
    Env { repo, bare }
}

fn screen(e: &Env) -> Runtime<State> {
    let mut rt = runtime(super::opened(e.repo.path()));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    rt.state.select_branch("main");
    for _ in 0..6 {
        step(&mut rt);
    }
    click(&mut rt, "branch-publish");
    settle(&mut rt, |s| {
        s.publishing
            .state
            .as_ref()
            .is_some_and(|p| p.preview.is_some())
    });
    rt
}

fn finished(s: &State) -> bool {
    s.publishing
        .state
        .as_ref()
        .is_some_and(|p| p.running.is_none() && (p.done.is_some() || p.error.is_some()))
}

fn remote_tip(e: &Env) -> String {
    git(e.bare.path(), &["rev-parse", "refs/heads/main"])
}

#[test]
fn a_new_branch_is_published_to_an_explicit_destination_and_verified() {
    let e = env();
    let mut rt = screen(&e);
    assert!(shows(&rt, "Destination : origin → refs/heads/main"));
    assert!(shows(&rt, "Nouvelle branche sur origin"));
    click(&mut rt, "publish-confirm");
    settle(&mut rt, finished);
    assert!(shows(
        &rt,
        "la référence distante a été relue et correspond"
    ));
    assert_eq!(remote_tip(&e), git(e.repo.path(), &["rev-parse", "HEAD"]));
    assert_eq!(
        git(e.repo.path(), &["config", "branch.main.remote"]),
        "origin"
    );
}

#[test]
fn a_remote_that_moved_refuses_and_the_message_explains_the_next_step() {
    let e = env();
    let mut rt = screen(&e);
    click(&mut rt, "publish-confirm");
    settle(&mut rt, finished);
    let clone = tempfile::tempdir().unwrap();
    git(
        clone.path(),
        &["clone", "-q", &e.bare.path().display().to_string(), "."],
    );
    std::fs::write(clone.path().join("o.txt"), "o\n").unwrap();
    git(clone.path(), &["add", "-A"]);
    git(clone.path(), &["commit", "-q", "-m", "theirs"]);
    git(clone.path(), &["push", "-q", "origin", "main"]);
    let theirs = remote_tip(&e);
    std::fs::write(e.repo.path().join("m.txt"), "m\n").unwrap();
    git(e.repo.path(), &["add", "-A"]);
    git(e.repo.path(), &["commit", "-q", "-m", "mine"]);
    rt.state.publish_close();
    click(&mut rt, "branch-publish");
    settle(&mut rt, |s| {
        s.publishing
            .state
            .as_ref()
            .is_some_and(|p| p.preview.is_some())
    });
    click(&mut rt, "publish-confirm");
    settle(&mut rt, finished);
    assert!(shows(&rt, "récupérez"));
    assert_eq!(remote_tip(&e), theirs);
}

#[test]
fn a_rewritten_history_needs_the_protected_confirmation_listing_what_is_replaced() {
    let e = env();
    let mut rt = screen(&e);
    click(&mut rt, "publish-confirm");
    settle(&mut rt, finished);
    let expected = git(e.repo.path(), &["rev-parse", "refs/remotes/origin/main"]);
    git(
        e.repo.path(),
        &["commit", "-q", "--amend", "-m", "first, reworded"],
    );
    rt.state.publish_close();
    click(&mut rt, "branch-publish");
    settle(&mut rt, |s| {
        s.publishing
            .state
            .as_ref()
            .is_some_and(|p| p.preview.is_some())
    });
    assert!(shows(&rt, "ont divergé"));
    assert!(rect_of(&rt, "publish-confirm").is_none());
    click(&mut rt, "publish-force");
    settle(&mut rt, |s| {
        s.publishing.state.as_ref().is_some_and(|p| p.confirm_force)
    });
    assert!(shows(&rt, &expected.chars().take(10).collect::<String>()));
    assert!(shows(&rt, "first"));
    click(&mut rt, "publish-force-confirm");
    settle(&mut rt, finished);
    assert_eq!(remote_tip(&e), git(e.repo.path(), &["rev-parse", "HEAD"]));
}

#[test]
fn without_any_remote_the_panel_says_how_to_add_one() {
    let e = env();
    git(e.repo.path(), &["remote", "remove", "origin"]);
    let mut rt = runtime(super::opened(e.repo.path()));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    rt.state.select_branch("main");
    for _ in 0..6 {
        step(&mut rt);
    }
    click(&mut rt, "branch-publish");
    settle(&mut rt, |s| {
        s.publishing
            .state
            .as_ref()
            .is_some_and(|p| p.error.is_some())
    });
    assert!(shows(&rt, "Aucun dépôt distant n'est configuré"));
    assert!(rect_of(&rt, "publish-confirm").is_none());
}
