use std::path::{Path, PathBuf};
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

fn idle(s: &State) -> bool {
    !s.worktrees.busy && !s.worktrees.loading
}

fn click(rt: &mut Runtime<State>, key: &str) {
    if key.starts_with("wt-") && !key.starts_with("wt-row") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("wt-panel", target));
        for _ in 0..8 {
            step(rt);
        }
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

struct Env {
    repo: tempfile::TempDir,
    area: tempfile::TempDir,
}

fn env() -> Env {
    let repo = tempfile::tempdir().unwrap();
    let p = repo.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "one"]);
    git(p, &["branch", "feature"]);
    Env {
        repo,
        area: tempfile::tempdir().unwrap(),
    }
}

fn screen(e: &Env) -> Runtime<State> {
    let mut rt = runtime(super::opened(e.repo.path()));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    super::menu(&mut rt, "nav-worktrees");
    settle(&mut rt, |s| s.worktrees.open && idle(s));
    rt
}

fn path_of(e: &Env, name: &str) -> PathBuf {
    std::fs::canonicalize(e.area.path()).unwrap().join(name)
}

#[test]
fn a_worktree_is_created_listed_with_its_own_state_and_opened() {
    let e = env();
    let mut rt = screen(&e);
    let wt = path_of(&e, "wt");
    rt.state.worktrees.new_path = TextEdit::new(wt.display().to_string());
    rt.state.worktrees.new_branch = TextEdit::new("topic");
    step(&mut rt);
    click(&mut rt, "wt-add");
    settle(&mut rt, |s| idle(s) && s.worktrees.list.len() == 2);
    assert!(wt.join("a.txt").exists());
    std::fs::write(wt.join("a.txt"), "edited in the worktree\n").unwrap();
    rt.state.reload_worktrees();
    settle(&mut rt, idle);
    assert!(shows(&rt, "topic"));
    assert!(shows(&rt, "1 modifié"));
    assert_eq!(
        std::fs::read_to_string(e.repo.path().join("a.txt")).unwrap(),
        "one\n"
    );
    rt.state.select_worktree(wt.clone());
    step(&mut rt);
    click(&mut rt, "wt-open");
    settle(&mut rt, |s| s.repo.as_ref().is_some_and(|r| r.name == "wt"));
    assert_eq!(
        rt.state
            .repo
            .as_ref()
            .unwrap()
            .snapshot
            .branch
            .head
            .as_deref(),
        Some("topic")
    );
}

#[test]
fn a_branch_already_checked_out_elsewhere_is_refused_with_the_path() {
    let e = env();
    let mut rt = screen(&e);
    rt.state.worktrees.new_path = TextEdit::new(path_of(&e, "dup").display().to_string());
    rt.state.worktrees.new_branch = TextEdit::new("main");
    rt.state.worktrees.create_branch = false;
    step(&mut rt);
    click(&mut rt, "wt-add");
    settle(&mut rt, |s| {
        idle(s)
            && s.notice
                .as_deref()
                .is_some_and(|n| n.contains("déjà extraite"))
    });
    assert!(!path_of(&e, "dup").exists());
    assert_eq!(rt.state.worktrees.list.len(), 1);
}

#[test]
fn removing_a_modified_worktree_names_the_files_and_needs_the_second_decision() {
    let e = env();
    let wt = path_of(&e, "work");
    git(
        e.repo.path(),
        &["worktree", "add", &wt.display().to_string(), "feature"],
    );
    std::fs::write(wt.join("a.txt"), "precious\n").unwrap();
    std::fs::write(wt.join("scratch.txt"), "scratch\n").unwrap();
    let mut rt = screen(&e);
    rt.state.select_worktree(wt.clone());
    step(&mut rt);
    click(&mut rt, "wt-remove");
    settle(&mut rt, |s| s.worktrees.removal.is_some());
    assert!(shows(&rt, "du travail non enregistré"));
    assert!(shows(&rt, "scratch.txt"));
    assert!(rect_of(&rt, "wt-remove-confirm").is_none());
    press(&mut rt, rune::Key::Escape);
    assert!(rt.state.worktrees.removal.is_none() && rt.state.worktrees.open);
    assert!(wt.exists());
    click(&mut rt, "wt-remove");
    settle(&mut rt, |s| s.worktrees.removal.is_some());
    click(&mut rt, "wt-remove-force");
    settle(&mut rt, |s| idle(s) && s.worktrees.list.len() == 1);
    assert!(!wt.exists());
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("sauvegardées sous refs/gitpoulpe/recovery"))
    );
    assert!(
        rt.state.notice.as_deref().is_some_and(
            |n| n.contains("1 fichier non suivi a été effacé et n'est pas récupérable")
        )
    );
}

#[test]
fn a_locked_worktree_cannot_be_removed_until_unlocked() {
    let e = env();
    let wt = path_of(&e, "locked");
    git(
        e.repo.path(),
        &["worktree", "add", &wt.display().to_string(), "feature"],
    );
    let mut rt = screen(&e);
    rt.state.select_worktree(wt.clone());
    step(&mut rt);
    rt.state.worktrees.lock_reason = TextEdit::new("keep");
    step(&mut rt);
    click(&mut rt, "wt-lock");
    settle(&mut rt, |s| {
        idle(s) && s.worktrees.list.iter().any(|w| w.locked.is_some())
    });
    assert!(shows(&rt, "Verrouillé : keep"));
    click(&mut rt, "wt-remove");
    settle(&mut rt, |s| s.worktrees.removal.is_some());
    assert!(shows(&rt, "déverrouillez-le d'abord"));
    click(&mut rt, "wt-remove-cancel");
    step(&mut rt);
    click(&mut rt, "wt-unlock");
    settle(&mut rt, |s| {
        idle(s) && s.worktrees.list.iter().all(|w| w.locked.is_none())
    });
    assert!(wt.exists());
}
