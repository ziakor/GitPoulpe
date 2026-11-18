use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use rune::Runtime;

use super::committing::{commits, git_text, settle_commit, staged_runtime};
use super::{click_at, rect_of, step};
use crate::state::State;

fn toggle_amend(rt: &mut Runtime<State>) {
    let toggle = rect_of(rt, "amend-toggle").expect("the amend toggle is on screen");
    click_at(rt, toggle[0] + 10.0, toggle[1] + toggle[3] / 2.0, false);
    for _ in 0..60 {
        step(rt);
        if rt.state.amend_info.is_some() || !rt.state.amend_mode {
            break;
        }
    }
    for _ in 0..4 {
        step(rt);
    }
}

fn press_commit_button(rt: &mut Runtime<State>) {
    let button = rect_of(rt, "commit").expect("the commit button is on screen");
    click_at(rt, button[0] + 10.0, button[1] + button[3] / 2.0, false);
    settle_commit(rt);
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

#[test]
fn amending_prefills_the_message_replaces_the_tip_and_records_the_previous_commit() {
    let (repo, mut rt) = staged_runtime(3);
    let dir = repo.path();
    let count = commits(dir);
    let tip = git_text(dir, &["rev-parse", "HEAD"]);
    let author = git_text(dir, &["log", "-1", "--format=%an <%ae>"]);

    toggle_amend(&mut rt);
    assert!(rt.state.amend_mode);
    assert_eq!(
        rt.state.commit_message.text, "base",
        "the last message is offered"
    );
    rt.state.commit_message.text = "base, reworded".to_owned();
    step(&mut rt);
    press_commit_button(&mut rt);

    assert_eq!(commits(dir), count, "the tip was replaced, not added to");
    assert_eq!(
        git_text(dir, &["log", "-1", "--format=%s"]),
        "base, reworded"
    );
    assert_eq!(
        git_text(dir, &["log", "-1", "--format=%an <%ae>"]),
        author,
        "author kept"
    );
    assert_eq!(
        git_text(dir, &["log", "-1", "--format=%cn <%ce>"]),
        "Ada Lovelace <ada@example.org>",
        "committer is the profile"
    );
    assert!(!rt.state.amend_mode, "the amend mode is left after success");
    assert!(rt.state.commit_message.text.is_empty());
    let notice = rt.state.notice.clone().unwrap_or_default();
    assert!(
        notice.contains("amendé") && notice.contains(&tip[..8]),
        "{notice}"
    );
    let journal = rt.state.journal();
    let last = journal.last().unwrap();
    assert_eq!(last.action, "amend");
    assert_eq!(last.before.as_deref(), Some(tip.as_str()));
    assert_eq!(
        git_text(dir, &["cat-file", "-t", &tip]),
        "commit",
        "the previous tip can still be recovered"
    );
}

#[test]
fn staged_changes_are_folded_into_the_amended_commit() {
    let (repo, mut rt) = staged_runtime(3);
    let dir = repo.path();
    let count = commits(dir);
    assert_eq!(
        rt.state.staged_for_commit().len(),
        1,
        "staged.txt is staged"
    );

    toggle_amend(&mut rt);
    press_commit_button(&mut rt);

    assert_eq!(commits(dir), count);
    assert_eq!(
        git_text(dir, &["show", "HEAD:staged.txt"]),
        "a\nb",
        "the staged content is part of the amended commit"
    );
    assert!(rt.state.staged_for_commit().is_empty());
}

fn push_to_a_remote(dir: &std::path::Path) -> tempfile::TempDir {
    let bare = tempfile::tempdir().unwrap();
    Command::new("git")
        .current_dir(bare.path())
        .args(["init", "-q", "--bare", "-b", "main"])
        .output()
        .unwrap();
    for args in [
        vec!["remote", "add", "origin", bare.path().to_str().unwrap()],
        vec!["push", "-q", "origin", "main"],
    ] {
        let status = Command::new("git")
            .current_dir(dir)
            .args(&args)
            .status()
            .unwrap();
        assert!(status.success(), "{args:?}");
    }
    bare
}

#[test]
fn a_published_commit_shows_the_consequences_and_needs_a_second_explicit_click() {
    let (repo, mut rt) = staged_runtime(3);
    let dir = repo.path();
    let _remote = push_to_a_remote(dir);
    let tip = git_text(dir, &["rev-parse", "HEAD"]);

    toggle_amend(&mut rt);
    let info = rt.state.amend_info.clone().unwrap();
    assert_eq!(info.published_on, vec!["origin/main".to_owned()]);
    assert!(
        shows(&rt, "déjà publié sur origin/main"),
        "the warning is visible before any confirmation"
    );
    assert!(shows(&rt, "réécrit l'historique"));

    rt.state.commit_message.text = "rewritten after publication".to_owned();
    step(&mut rt);
    press_commit_button(&mut rt);
    assert_eq!(
        git_text(dir, &["rev-parse", "HEAD"]),
        tip,
        "first click only asks for confirmation"
    );
    assert!(rt.state.amend_confirming);
    assert!(shows(&rt, "pour confirmer"));

    press_commit_button(&mut rt);
    assert_ne!(
        git_text(dir, &["rev-parse", "HEAD"]),
        tip,
        "second click amends"
    );
    assert_eq!(
        git_text(dir, &["log", "-1", "--format=%s"]),
        "rewritten after publication"
    );
    assert_eq!(git_text(dir, &["cat-file", "-t", &tip]), "commit");
}

#[test]
fn an_amendment_that_changes_nothing_is_refused_and_the_mode_stays_open() {
    let (repo, mut rt) = staged_runtime(3);
    let dir = repo.path();
    Command::new("git")
        .current_dir(dir)
        .args(["reset", "-q"])
        .status()
        .unwrap();
    rt.state.refresh();
    for _ in 0..30 {
        step(&mut rt);
    }
    let tip = git_text(dir, &["rev-parse", "HEAD"]);

    toggle_amend(&mut rt);
    press_commit_button(&mut rt);

    assert_eq!(git_text(dir, &["rev-parse", "HEAD"]), tip);
    let notice = rt.state.notice.clone().unwrap_or_default();
    assert!(notice.contains("Rien à amender"), "{notice}");
    assert!(rt.state.amend_mode, "the user can still edit the message");
}

#[test]
fn a_refusing_hook_leaves_the_tip_and_the_message_and_the_retry_works() {
    let (repo, mut rt) = staged_runtime(3);
    let dir = repo.path();
    let tip = git_text(dir, &["rev-parse", "HEAD"]);
    let hook = dir.join(".git/hooks/commit-msg");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(
        &hook,
        "#!/bin/sh\necho 'message rejected by policy' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    toggle_amend(&mut rt);
    rt.state.commit_message.text = "policy test".to_owned();
    step(&mut rt);
    press_commit_button(&mut rt);
    assert_eq!(
        git_text(dir, &["rev-parse", "HEAD"]),
        tip,
        "the previous tip is still the tip"
    );
    let report = rt.state.commit_report.clone().unwrap_or_default();
    assert!(report.contains("message rejected by policy"), "{report}");
    assert_eq!(rt.state.commit_message.text, "policy test");
    assert!(rt.state.amend_mode);

    std::fs::remove_file(&hook).unwrap();
    press_commit_button(&mut rt);
    assert_ne!(git_text(dir, &["rev-parse", "HEAD"]), tip);
}

#[test]
fn leaving_the_amend_mode_clears_a_message_that_was_only_prefilled() {
    let (_repo, mut rt) = staged_runtime(3);
    toggle_amend(&mut rt);
    assert_eq!(rt.state.commit_message.text, "base");
    toggle_amend(&mut rt);
    assert!(!rt.state.amend_mode);
    assert!(rt.state.commit_message.text.is_empty());

    rt.state.commit_message.text = "my own draft".to_owned();
    toggle_amend(&mut rt);
    assert_eq!(
        rt.state.commit_message.text, "my own draft",
        "a typed draft is never overwritten"
    );
}
