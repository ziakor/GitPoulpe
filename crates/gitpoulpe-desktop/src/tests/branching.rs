use std::path::Path;
use std::process::Command;

use rune::{Runtime, TextEdit};

use super::{click_at, press, rect_of, runtime, step};
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
    assert!(output.status.success(), "{args:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn settle(rt: &mut Runtime<State>, until: impl Fn(&State) -> bool) {
    for _ in 0..80 {
        step(rt);
        if until(&rt.state) {
            break;
        }
    }
    for _ in 0..4 {
        step(rt);
    }
    assert!(until(&rt.state), "the screen never settled");
}

fn idle(s: &State) -> bool {
    !s.branches.busy && !s.branches.loading
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "base\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "base"]);
    git(p, &["switch", "-q", "-c", "wip"]);
    std::fs::write(p.join("a.txt"), "wip edit\n").unwrap();
    git(p, &["commit", "-qam", "unmerged work"]);
    git(p, &["switch", "-q", "main"]);
    git(p, &["branch", "merged"]);
    dir
}

fn screen(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && idle(s));
    rt
}

fn pick(rt: &mut Runtime<State>, name: &str) {
    rt.state.select_branch(name);
    for _ in 0..6 {
        step(rt);
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = git(
        dir,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )
    .lines()
    .map(str::to_owned)
    .collect();
    v.sort();
    v
}

#[test]
fn the_branch_screen_lists_branches_and_marks_the_current_one() {
    let dir = repo();
    let rt = screen(dir.path());
    let listed: Vec<String> = rt
        .state
        .branches
        .branches
        .iter()
        .map(|b| b.name.clone())
        .collect();
    assert_eq!(listed, names(dir.path()));
    assert!(
        rt.state
            .branches
            .branches
            .iter()
            .find(|b| b.name == "main")
            .unwrap()
            .current
    );
    assert!(shows(&rt, "courante"));
    assert!(shows(&rt, "unmerged work"));
}

#[test]
fn creating_selecting_and_renaming_a_branch_updates_git_and_the_screen() {
    let dir = repo();
    let mut rt = screen(dir.path());
    rt.state.branches.new_name = TextEdit::new("topic");
    click(&mut rt, "branch-create");
    settle(&mut rt, |s| {
        idle(s) && s.branches.branches.iter().any(|b| b.name == "topic")
    });
    assert!(names(dir.path()).contains(&"topic".to_owned()));
    assert_eq!(git(dir.path(), &["branch", "--show-current"]), "main");
    assert_eq!(rt.state.branches.new_name.text, "");

    pick(&mut rt, "topic");
    click(&mut rt, "branch-switch");
    settle(&mut rt, |s| {
        idle(s) && s.repo.as_ref().unwrap().snapshot.branch.head.as_deref() == Some("topic")
    });
    assert_eq!(git(dir.path(), &["branch", "--show-current"]), "topic");

    rt.state.branches.rename_to = TextEdit::new("renamed");
    click(&mut rt, "branch-rename");
    settle(&mut rt, |s| {
        idle(s) && s.branches.branches.iter().any(|b| b.name == "renamed")
    });
    assert_eq!(git(dir.path(), &["branch", "--show-current"]), "renamed");
    assert!(!names(dir.path()).contains(&"topic".to_owned()));
    let journal = rt.state.journal();
    assert!(journal.iter().any(|e| e.action == "switch"));
    assert!(
        journal
            .iter()
            .any(|e| e.action == "branch-rename" && e.detail == "topic -> renamed")
    );
}

#[test]
fn an_invalid_name_is_reported_and_creates_nothing() {
    let dir = repo();
    let before = names(dir.path());
    let mut rt = screen(dir.path());
    rt.state.branches.new_name = TextEdit::new("bad name..x");
    click(&mut rt, "branch-create");
    settle(&mut rt, idle);
    assert_eq!(names(dir.path()), before);
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("nom de branche invalide"))
    );
}

#[test]
fn a_switch_that_would_overwrite_local_changes_is_refused_and_the_file_survives() {
    let dir = repo();
    std::fs::write(dir.path().join("a.txt"), "uncommitted work\n").unwrap();
    let mut rt = screen(dir.path());
    pick(&mut rt, "wip");
    click(&mut rt, "branch-switch");
    settle(&mut rt, |s| idle(s) && s.notice.is_some());
    assert_eq!(git(dir.path(), &["branch", "--show-current"]), "main");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "uncommitted work\n"
    );
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("would be overwritten")),
        "{:?}",
        rt.state.notice
    );
}

#[test]
fn deleting_an_unmerged_branch_asks_first_and_names_the_lost_commits() {
    let dir = repo();
    let tip = git(dir.path(), &["rev-parse", "wip"]);
    let mut rt = screen(dir.path());
    pick(&mut rt, "wip");
    click(&mut rt, "branch-delete");
    settle(&mut rt, |s| s.branches.confirm.is_some());
    assert!(
        names(dir.path()).contains(&"wip".to_owned()),
        "nothing is deleted before the confirmation"
    );
    assert!(shows(&rt, "perdre 1 commit"));
    assert!(shows(&rt, "unmerged work"));
    assert!(shows(&rt, &format!("git branch -- 'wip' {tip}")));

    click(&mut rt, "branch-delete-cancel");
    settle(&mut rt, |s| s.branches.confirm.is_none());
    assert!(names(dir.path()).contains(&"wip".to_owned()));

    click(&mut rt, "branch-delete");
    settle(&mut rt, |s| s.branches.confirm.is_some());
    click(&mut rt, "branch-delete-force");
    settle(&mut rt, |s| {
        idle(s) && !s.branches.branches.iter().any(|b| b.name == "wip")
    });
    assert!(!names(dir.path()).contains(&"wip".to_owned()));
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains(&format!("git branch -- 'wip' {tip}")))
    );
    let entry = rt
        .state
        .journal()
        .into_iter()
        .find(|e| e.action == "branch-delete")
        .expect("the deletion is journaled");
    assert_eq!(entry.before.as_deref(), Some(tip.as_str()));
    git(dir.path(), &["branch", "wip", &tip]);
}

#[test]
fn a_merged_branch_is_deleted_without_a_confirmation() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "merged");
    click(&mut rt, "branch-delete");
    settle(&mut rt, |s| {
        idle(s) && !s.branches.branches.iter().any(|b| b.name == "merged")
    });
    assert!(!names(dir.path()).contains(&"merged".to_owned()));
    assert!(rt.state.branches.confirm.is_none());
}

#[test]
fn the_current_branch_and_a_branch_of_another_worktree_offer_no_switch_or_delete() {
    let dir = repo();
    let work = tempfile::tempdir().unwrap();
    let tree = work.path().join("wt");
    git(
        dir.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "busy",
            tree.to_str().unwrap(),
        ],
    );
    let mut rt = screen(dir.path());
    pick(&mut rt, "main");
    assert!(rect_of(&rt, "branch-switch").is_none() && rect_of(&rt, "branch-delete").is_none());
    pick(&mut rt, "busy");
    assert!(rect_of(&rt, "branch-switch").is_none() && rect_of(&rt, "branch-delete").is_none());
    assert!(shows(&rt, "déjà extraite dans le worktree"));
    pick(&mut rt, "merged");
    assert!(rect_of(&rt, "branch-switch").is_some() && rect_of(&rt, "branch-delete").is_some());
}

#[test]
fn escape_cancels_a_pending_deletion_first_then_closes_the_screen() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "wip");
    click(&mut rt, "branch-delete");
    settle(&mut rt, |s| s.branches.confirm.is_some());
    press(&mut rt, rune::Key::Escape);
    assert!(rt.state.branches.confirm.is_none() && rt.state.branches.open);
    press(&mut rt, rune::Key::Escape);
    assert!(!rt.state.branches.open);
    assert!(names(dir.path()).contains(&"wip".to_owned()));
}
