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
    if key.starts_with("undo-") || key.starts_with("redo-") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("undo-panel", target));
        for _ in 0..8 {
            step(rt);
        }
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "one"]);
    dir
}

fn branches(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    rt
}

fn open_undo(rt: &mut Runtime<State>) {
    super::menu(rt, "nav-undo");
    settle(rt, |s| s.undo_screen.open);
}

fn create_branch(rt: &mut Runtime<State>, name: &str) {
    rt.state.branches.new_name = TextEdit::new(name);
    step(rt);
    click(rt, "branch-create");
    settle(rt, |s| {
        !s.branches.busy
            && !s.branches.loading
            && s.branches.branches.iter().any(|b| b.name == name)
    });
}

#[test]
fn a_branch_creation_is_listed_undone_and_redone() {
    let dir = repo();
    let mut rt = branches(dir.path());
    create_branch(&mut rt, "feature");
    open_undo(&mut rt);
    assert!(shows(&rt, "Branche créée : feature"));
    let id = rt.state.undo_records().last().unwrap().id;
    rt.state.select_undo(id);
    step(&mut rt);
    click(&mut rt, "undo-run");
    settle(&mut rt, |s| {
        !s.undo_screen.busy && s.undo_screen.message.as_deref() == Some("Opération annulée.")
    });
    assert!(git(dir.path(), &["for-each-ref", "refs/heads/feature"]).is_empty());
    assert!(shows(&rt, "annulée"));
    click(&mut rt, "redo-run");
    settle(&mut rt, |s| {
        !s.undo_screen.busy && s.undo_screen.message.as_deref() == Some("Opération rétablie.")
    });
    assert!(git(dir.path(), &["for-each-ref", "refs/heads/feature"]).contains("feature"));
}

#[test]
fn a_change_made_outside_blocks_the_undo_with_an_explanation() {
    let dir = repo();
    let mut rt = branches(dir.path());
    create_branch(&mut rt, "feature");
    std::fs::write(dir.path().join("b.txt"), "b\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "second"]);
    git(dir.path(), &["branch", "-f", "feature", "HEAD"]);
    open_undo(&mut rt);
    let id = rt.state.undo_records().last().unwrap().id;
    rt.state.select_undo(id);
    step(&mut rt);
    click(&mut rt, "undo-run");
    settle(&mut rt, |s| {
        !s.undo_screen.busy && s.undo_screen.message.is_some()
    });
    assert!(shows(&rt, "l'état a changé depuis l'opération"));
    assert!(git(dir.path(), &["for-each-ref", "refs/heads/feature"]).contains("feature"));
}

#[test]
fn a_merge_is_undone_by_moving_the_branch_back_and_unsupported_operations_are_explained() {
    let dir = repo();
    let p = dir.path();
    git(p, &["checkout", "-q", "-b", "side"]);
    std::fs::write(p.join("s.txt"), "s\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "side work"]);
    git(p, &["checkout", "-q", "main"]);
    let before = git(p, &["rev-parse", "HEAD"]);
    let mut rt = branches(p);
    git(p, &["merge", "-q", "--ff-only", "side"]);
    let after = git(p, &["rev-parse", "HEAD"]);
    rt.state.record_journal(
        "merge",
        Some(before.clone()),
        Some(after),
        "side".to_owned(),
    );
    rt.state.record_journal(
        "push",
        Some(before.clone()),
        Some(before.clone()),
        "origin main".to_owned(),
    );
    open_undo(&mut rt);
    assert!(shows(&rt, "non réversible"));
    let push_id = rt.state.undo_records().last().unwrap().id;
    rt.state.select_undo(push_id);
    step(&mut rt);
    assert!(shows(&rt, "le distant a déjà reçu les commits"));
    assert!(rect_of(&rt, "undo-run").is_none() && rect_of(&rt, "redo-run").is_none());
    let merge_id = rt
        .state
        .undo_records()
        .iter()
        .find(|r| r.label.starts_with("Fusion"))
        .unwrap()
        .id;
    rt.state.select_undo(merge_id);
    step(&mut rt);
    click(&mut rt, "undo-run");
    settle(&mut rt, |s| {
        !s.undo_screen.busy && s.undo_screen.message.as_deref() == Some("Opération annulée.")
    });
    assert_eq!(git(p, &["rev-parse", "HEAD"]), before);
    assert!(!p.join("s.txt").exists());
}

#[test]
fn a_repository_without_recorded_operations_says_so() {
    let dir = repo();
    let mut rt = branches(dir.path());
    open_undo(&mut rt);
    assert!(shows(&rt, "Aucune opération enregistrée"));
}
