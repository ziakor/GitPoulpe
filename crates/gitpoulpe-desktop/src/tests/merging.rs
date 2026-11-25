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

fn idle(s: &State) -> bool {
    !s.branches.loading && s.branches.merge.as_ref().is_none_or(|m| !m.busy)
}

fn click(rt: &mut Runtime<State>, key: &str) {
    if key.starts_with("merge-") || key == "branch-merge" {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("branch-panel", target));
        for _ in 0..8 {
            step(rt);
        }
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn screen(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && idle(s));
    rt
}

fn open_preview(rt: &mut Runtime<State>, source: &str) {
    rt.state.select_branch(source);
    for _ in 0..6 {
        step(rt);
    }
    click(rt, "branch-merge");
    settle(rt, |s| {
        s.branches
            .merge
            .as_ref()
            .is_some_and(|m| m.preview.is_some())
    });
}

fn base(p: &Path) {
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "base\n").unwrap();
    commit_all(p, "base");
}

fn linear() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    base(p);
    git(p, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(p.join("f.txt"), "f\n").unwrap();
    commit_all(p, "feature work");
    git(p, &["checkout", "-q", "main"]);
    dir
}

fn divergent() -> tempfile::TempDir {
    let dir = linear();
    std::fs::write(dir.path().join("m.txt"), "m\n").unwrap();
    commit_all(dir.path(), "main work");
    dir
}

fn conflicting() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    base(p);
    git(p, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(p.join("a.txt"), "feature\n").unwrap();
    commit_all(p, "feature");
    git(p, &["checkout", "-q", "main"]);
    std::fs::write(p.join("a.txt"), "main\n").unwrap();
    commit_all(p, "main");
    dir
}

#[test]
fn the_preview_names_both_branches_and_a_linear_merge_fast_forwards() {
    let dir = linear();
    let mut rt = screen(dir.path());
    open_preview(&mut rt, "feature");
    assert!(shows(&rt, "Fusionner « feature » dans « main »"));
    assert!(shows(&rt, "Avance rapide"));
    click(&mut rt, "merge-confirm");
    settle(&mut rt, |s| s.branches.merge.is_none() && idle(s));
    assert_eq!(
        git(dir.path(), &["rev-parse", "HEAD"]),
        git(dir.path(), &["rev-parse", "feature"])
    );
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("avance rapide"))
    );
}

#[test]
fn a_divergent_merge_creates_a_merge_commit_and_the_option_forces_one_when_linear() {
    let dir = divergent();
    let mut rt = screen(dir.path());
    open_preview(&mut rt, "feature");
    assert!(shows(&rt, "Commit de fusion"));
    assert!(shows(&rt, "Aucun conflit prévu"));
    click(&mut rt, "merge-confirm");
    settle(&mut rt, |s| s.branches.merge.is_none() && idle(s));
    assert_eq!(
        git(dir.path(), &["rev-list", "--parents", "-n", "1", "HEAD"])
            .split(' ')
            .count(),
        3
    );
    let linear_dir = linear();
    let mut rt = screen(linear_dir.path());
    open_preview(&mut rt, "feature");
    click(&mut rt, "merge-no-ff");
    step(&mut rt);
    assert!(super::ticked(&rt, "merge-no-ff"));
    click(&mut rt, "merge-confirm");
    settle(&mut rt, |s| s.branches.merge.is_none() && idle(s));
    assert_eq!(
        git(
            linear_dir.path(),
            &["rev-list", "--parents", "-n", "1", "HEAD"]
        )
        .split(' ')
        .count(),
        3
    );
}

#[test]
fn an_already_merged_branch_offers_nothing_to_merge() {
    let dir = linear();
    git(dir.path(), &["merge", "-q", "feature"]);
    let mut rt = screen(dir.path());
    open_preview(&mut rt, "feature");
    assert!(shows(&rt, "rien à fusionner"));
    assert!(rect_of(&rt, "merge-confirm").is_none());
}

#[test]
fn predicted_conflicts_are_shown_and_a_conflicting_merge_opens_the_resolution() {
    let dir = conflicting();
    let mut rt = screen(dir.path());
    open_preview(&mut rt, "feature");
    assert!(shows(&rt, "1 fichier en conflit prévu : a.txt"));
    click(&mut rt, "merge-confirm");
    settle(&mut rt, |s| s.conflicts.open && s.operation_kind.is_some());
    assert_eq!(rt.state.conflicts.files.len(), 1);
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("conflits"))
    );
    assert!(super::head_context(&rt, "conflicts-head").starts_with("fusion · "));
}

#[test]
fn local_work_that_would_be_overwritten_blocks_the_merge_until_it_is_set_aside() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    base(p);
    git(p, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(p.join("a.txt"), "feature\n").unwrap();
    commit_all(p, "feature");
    git(p, &["checkout", "-q", "main"]);
    std::fs::write(p.join("b.txt"), "b\n").unwrap();
    commit_all(p, "main");
    std::fs::write(p.join("a.txt"), "my unsaved work\n").unwrap();
    let head = git(p, &["rev-parse", "HEAD"]);
    let mut rt = screen(p);
    open_preview(&mut rt, "feature");
    assert!(shows(&rt, "1 fichier a des modifications locales (a.txt)"));
    click(&mut rt, "merge-confirm");
    settle(&mut rt, |s| {
        s.branches
            .merge
            .as_ref()
            .is_some_and(|m| m.error.is_some() && !m.busy)
    });
    assert!(shows(&rt, "seraient écrasées"));
    assert_eq!(
        std::fs::read_to_string(p.join("a.txt")).unwrap(),
        "my unsaved work\n"
    );
    assert_eq!(git(p, &["rev-parse", "HEAD"]), head);
    click(&mut rt, "merge-autostash");
    step(&mut rt);
    click(&mut rt, "merge-confirm");
    settle(&mut rt, |s| s.branches.merge.is_none() && idle(s));
    assert_eq!(git(p, &["stash", "list"]).lines().count(), 1);
    assert!(git(p, &["stash", "show", "-p", "stash@{0}"]).contains("my unsaved work"));
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("réserve"))
    );
}
