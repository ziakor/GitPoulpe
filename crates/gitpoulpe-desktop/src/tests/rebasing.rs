use std::path::Path;
use std::process::Command;

use rune::{Runtime, TextEdit};

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

fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
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
    if key.starts_with("rebase-") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("hist-panel", target));
        for _ in 0..8 {
            step(rt);
        }
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn repo() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("base.txt"), "base\n").unwrap();
    commit_all(p, "base");
    let base = git(p, &["rev-parse", "HEAD"]);
    for n in ["a", "b", "c"] {
        std::fs::write(p.join(format!("{n}.txt")), n).unwrap();
        commit_all(p, &format!("add {n}"));
    }
    (dir, base)
}

fn screen(dir: &Path, base: &str) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "history");
    settle(&mut rt, |s| s.history.commits.len() >= 3);
    let index = rt
        .state
        .history
        .commits
        .iter()
        .position(|c| c.oid == base)
        .unwrap();
    rt.state.select_commit(index);
    for _ in 0..6 {
        step(&mut rt);
    }
    click(&mut rt, "rebase-here");
    settle(&mut rt, |s| {
        s.rebasing.state.as_ref().is_some_and(|r| r.info.is_some())
    });
    rt
}

#[test]
fn the_plan_lists_the_base_and_commits_and_a_squash_rebase_runs() {
    let (dir, base) = repo();
    let mut rt = screen(dir.path(), &base);
    assert!(shows(&rt, "3 commit(s) réécrits"));
    assert!(shows(&rt, "add a") && shows(&rt, "add c"));
    click(&mut rt, "rebase-act1");
    for _ in 0..2 {
        click(&mut rt, "rebase-act1");
    }
    assert_eq!(
        rt.state.rebasing.state.as_ref().unwrap().items[1]
            .action
            .caption(),
        "squash"
    );
    click(&mut rt, "rebase-run");
    settle(&mut rt, |s| {
        s.rebasing.state.is_none()
            && s.notice
                .as_deref()
                .is_some_and(|n| n.contains("Rebase terminé"))
    });
    assert_eq!(
        git(
            dir.path(),
            &["rev-list", "--count", &format!("{base}..HEAD")]
        ),
        "2"
    );
    assert!(dir.path().join("b.txt").exists());
    assert_eq!(git(dir.path(), &["log", "-1", "--format=%s"]), "add c");
}

#[test]
fn an_incompatible_plan_is_flagged_and_cannot_be_run() {
    let (dir, base) = repo();
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    let mut rt = screen(dir.path(), &base);
    for _ in 0..3 {
        click(&mut rt, "rebase-act0");
    }
    assert_eq!(
        rt.state.rebasing.state.as_ref().unwrap().items[0]
            .action
            .caption(),
        "squash"
    );
    step(&mut rt);
    assert!(shows(&rt, "ne peut pas être « squash » ou « fixup »"));
    click(&mut rt, "rebase-run");
    for _ in 0..10 {
        step(&mut rt);
    }
    assert!(rt.state.rebasing.state.as_ref().is_some_and(|r| !r.busy));
    assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head);
}

#[test]
fn a_reworded_commit_takes_the_new_message() {
    let (dir, base) = repo();
    let mut rt = screen(dir.path(), &base);
    click(&mut rt, "rebase-act2");
    step(&mut rt);
    rt.state.rebasing.state.as_mut().unwrap().message = TextEdit::new("add c, better said");
    step(&mut rt);
    click(&mut rt, "rebase-run");
    settle(&mut rt, |s| s.rebasing.state.is_none());
    assert_eq!(
        git(dir.path(), &["log", "-1", "--format=%s"]),
        "add c, better said"
    );
}

#[test]
fn a_conflict_pauses_the_rebase_and_opens_the_resolution_with_the_old_tip_named() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("f.txt"), "zero\n").unwrap();
    commit_all(p, "base");
    let base = git(p, &["rev-parse", "HEAD"]);
    std::fs::write(p.join("f.txt"), "one\n").unwrap();
    commit_all(p, "first");
    std::fs::write(p.join("f.txt"), "two\n").unwrap();
    commit_all(p, "second");
    let old = git(p, &["rev-parse", "HEAD"]);
    let mut rt = screen(p, &base);
    click(&mut rt, "rebase-up1");
    step(&mut rt);
    click(&mut rt, "rebase-run");
    settle(&mut rt, |s| s.conflicts.open && s.operation_kind.is_some());
    assert!(super::head_context(&rt, "conflicts-head").starts_with("rebase · "));
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("refs/gitpoulpe/recovery"))
    );
    click(&mut rt, "cf-abort");
    settle(&mut rt, |s| s.conflicts.confirm_abort);
    click(&mut rt, "cf-abort-confirm-yes");
    settle(&mut rt, |s| !s.conflicts.open && s.operation_kind.is_none());
    assert_eq!(git(p, &["rev-parse", "HEAD"]), old);
}
