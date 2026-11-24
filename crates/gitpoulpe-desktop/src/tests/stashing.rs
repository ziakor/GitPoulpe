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
    !s.stashes.busy && !s.stashes.loading
}

fn click(rt: &mut Runtime<State>, key: &str) {
    if key.starts_with("st-") && !key.starts_with("st-row") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("st-panel", target));
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
    std::fs::write(p.join("a.txt"), "a base\n").unwrap();
    std::fs::write(p.join("b.txt"), "b base\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "base"]);
    dir
}

fn screen(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    super::menu(&mut rt, "nav-stashes");
    settle(&mut rt, |s| s.stashes.open && idle(s));
    rt
}

fn work(p: &Path) {
    std::fs::write(p.join("a.txt"), "a staged\n").unwrap();
    git(p, &["add", "a.txt"]);
    std::fs::write(p.join("b.txt"), "b unstaged\n").unwrap();
    std::fs::write(p.join("new.txt"), "untracked\n").unwrap();
}

#[test]
fn work_is_set_aside_with_the_chosen_scope_and_listed_with_its_content() {
    let dir = repo();
    work(dir.path());
    let mut rt = screen(dir.path());
    rt.state.stashes.message = TextEdit::new("before the demo");
    step(&mut rt);
    click(&mut rt, "st-untracked");
    step(&mut rt);
    assert!(shows(&rt, "[x] Inclure les fichiers non suivis"));
    click(&mut rt, "st-create");
    settle(&mut rt, |s| idle(s) && s.stashes.stashes.len() == 1);
    assert!(git(dir.path(), &["status", "--porcelain"]).is_empty());
    assert!(shows(&rt, "before the demo"));
    let oid = rt.state.stashes.stashes[0].oid.clone();
    rt.state.select_stash(&oid);
    settle(&mut rt, |s| s.stashes.detail.is_some());
    assert!(shows(&rt, "(indexé)"));
    assert!(shows(&rt, "new.txt  (non suivi)"));
}

#[test]
fn staged_only_excludes_the_other_options() {
    let dir = repo();
    work(dir.path());
    let mut rt = screen(dir.path());
    click(&mut rt, "st-untracked");
    click(&mut rt, "st-staged");
    step(&mut rt);
    assert!(rt.state.stashes.staged_only && !rt.state.stashes.include_untracked);
    click(&mut rt, "st-create");
    settle(&mut rt, |s| idle(s) && s.stashes.stashes.len() == 1);
    assert_eq!(
        git(dir.path(), &["status", "--porcelain"]).lines().count(),
        2
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "b unstaged\n"
    );
}

#[test]
fn applying_keeps_the_stash_and_popping_removes_it() {
    let dir = repo();
    std::fs::write(dir.path().join("b.txt"), "b changed\n").unwrap();
    let mut rt = screen(dir.path());
    click(&mut rt, "st-create");
    settle(&mut rt, |s| idle(s) && s.stashes.stashes.len() == 1);
    let oid = rt.state.stashes.stashes[0].oid.clone();
    rt.state.select_stash(&oid);
    settle(&mut rt, |s| s.stashes.detail.is_some());
    click(&mut rt, "st-apply");
    settle(&mut rt, |s| {
        idle(s) && std::fs::read_to_string(s_path(dir.path(), "b.txt")).unwrap() == "b changed\n"
    });
    assert_eq!(rt.state.stashes.stashes.len(), 1);
    git(dir.path(), &["checkout", "-q", "--", "b.txt"]);
    click(&mut rt, "st-pop");
    settle(&mut rt, |s| idle(s) && s.stashes.stashes.is_empty());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "b changed\n"
    );
}

fn s_path(dir: &Path, name: &str) -> std::path::PathBuf {
    dir.join(name)
}

#[test]
fn a_conflicting_application_keeps_the_stash_and_opens_the_resolution() {
    let dir = repo();
    std::fs::write(dir.path().join("a.txt"), "stashed\n").unwrap();
    let mut rt = screen(dir.path());
    click(&mut rt, "st-create");
    settle(&mut rt, |s| idle(s) && s.stashes.stashes.len() == 1);
    std::fs::write(dir.path().join("a.txt"), "committed later\n").unwrap();
    git(dir.path(), &["commit", "-qam", "diverge"]);
    let oid = rt.state.stashes.stashes[0].oid.clone();
    rt.state.select_stash(&oid);
    settle(&mut rt, |s| s.stashes.detail.is_some());
    click(&mut rt, "st-pop");
    settle(&mut rt, |s| s.conflicts.open && s.operation_kind.is_some());
    assert_eq!(git(dir.path(), &["stash", "list"]).lines().count(), 1);
    assert!(shows(&rt, "Résolution · application du stash"));
    assert!(shows(&rt, "Le stash est conservé"));
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("Le stash est conservé"))
    );
}

#[test]
fn dropping_asks_first_and_names_the_recovery_command() {
    let dir = repo();
    std::fs::write(dir.path().join("b.txt"), "precious\n").unwrap();
    let mut rt = screen(dir.path());
    rt.state.stashes.message = TextEdit::new("keep me");
    step(&mut rt);
    click(&mut rt, "st-create");
    settle(&mut rt, |s| idle(s) && s.stashes.stashes.len() == 1);
    let oid = rt.state.stashes.stashes[0].oid.clone();
    rt.state.select_stash(&oid);
    settle(&mut rt, |s| s.stashes.detail.is_some());
    click(&mut rt, "st-drop");
    settle(&mut rt, |s| s.stashes.confirm_drop.is_some());
    assert!(shows(&rt, &format!("git stash store -m 'keep me' {oid}")));
    press(&mut rt, rune::Key::Escape);
    assert!(rt.state.stashes.confirm_drop.is_none() && rt.state.stashes.open);
    assert_eq!(git(dir.path(), &["stash", "list"]).lines().count(), 1);
    click(&mut rt, "st-drop");
    settle(&mut rt, |s| s.stashes.confirm_drop.is_some());
    click(&mut rt, "st-drop-confirm");
    settle(&mut rt, |s| idle(s) && s.stashes.stashes.is_empty());
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("Pour le rétablir"))
    );
    git(dir.path(), &["stash", "store", "-m", "recovered", &oid]);
    assert_eq!(git(dir.path(), &["stash", "list"]).lines().count(), 1);
}

#[test]
fn nothing_to_stash_is_reported_without_creating_anything() {
    let dir = repo();
    let mut rt = screen(dir.path());
    click(&mut rt, "st-create");
    settle(&mut rt, |s| {
        idle(s)
            && s.notice
                .as_deref()
                .is_some_and(|n| n.contains("aucun changement"))
    });
    assert!(git(dir.path(), &["stash", "list"]).is_empty());
}
