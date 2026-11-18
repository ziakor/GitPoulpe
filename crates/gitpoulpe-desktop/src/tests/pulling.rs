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
    if key.starts_with("pull-") || key == "branch-pull" {
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

fn actor(e: &Env, file: &str, content: &str) -> String {
    let clone = tempfile::tempdir().unwrap();
    git(
        clone.path(),
        &["clone", "-q", &e.bare.path().display().to_string(), "."],
    );
    std::fs::write(clone.path().join(file), content).unwrap();
    git(clone.path(), &["add", "-A"]);
    git(
        clone.path(),
        &["commit", "-q", "-m", &format!("remote {file}")],
    );
    git(clone.path(), &["push", "-q", "origin", "main"]);
    git(clone.path(), &["rev-parse", "HEAD"])
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
    click(&mut rt, "branch-pull");
    settle(&mut rt, |s| {
        s.pulling
            .state
            .as_ref()
            .is_some_and(|p| p.preview.is_some())
    });
    rt
}

fn finished(s: &State) -> bool {
    s.pulling
        .state
        .as_ref()
        .is_none_or(|p| p.running.is_none() && (p.done.is_some() || p.error.is_some()))
}

#[test]
fn the_tracked_branch_and_the_strategy_are_shown_and_a_behind_branch_fast_forwards() {
    let e = env();
    let theirs = actor(&e, "theirs.txt", "t\n");
    git(e.repo.path(), &["fetch", "-q", "origin"]);
    let mut rt = screen(&e);
    assert!(shows(
        &rt,
        "Branche suivie : origin/main → main (en retard)"
    ));
    assert!(shows(&rt, "Stratégie choisie : avance rapide seulement"));
    click(&mut rt, "pull-run");
    settle(&mut rt, finished);
    assert!(shows(&rt, "Intégré par avance rapide"));
    assert_eq!(git(e.repo.path(), &["rev-parse", "HEAD"]), theirs);
}

#[test]
fn a_divergence_is_flagged_for_ff_only_and_the_chosen_strategy_is_the_one_used() {
    let e = env();
    actor(&e, "theirs.txt", "t\n");
    std::fs::write(e.repo.path().join("mine.txt"), "m\n").unwrap();
    git(e.repo.path(), &["add", "-A"]);
    git(e.repo.path(), &["commit", "-q", "-m", "mine"]);
    git(e.repo.path(), &["fetch", "-q", "origin"]);
    let mut rt = screen(&e);
    assert!(shows(&rt, "divergée"));
    assert!(shows(&rt, "l'avance rapide sera refusée"));
    click(&mut rt, "pull-run");
    settle(&mut rt, finished);
    assert!(shows(&rt, "l'avance rapide est impossible"));
    assert_eq!(git(e.repo.path(), &["log", "-1", "--format=%s"]), "mine");
    click(&mut rt, "pull-rebase");
    step(&mut rt);
    assert!(shows(&rt, "Stratégie choisie : rebase"));
    click(&mut rt, "pull-run");
    settle(&mut rt, |s| {
        s.notice.as_deref().is_some_and(|n| n.contains("rebase"))
    });
    assert_eq!(
        git(e.repo.path(), &["rev-list", "--merges", "--count", "HEAD"]),
        "0"
    );
    assert_eq!(git(e.repo.path(), &["log", "-1", "--format=%s"]), "mine");
}

#[test]
fn a_conflict_in_the_integration_is_not_hidden_and_opens_the_resolution() {
    let e = env();
    actor(&e, "a.txt", "theirs\n");
    std::fs::write(e.repo.path().join("a.txt"), "mine\n").unwrap();
    git(e.repo.path(), &["commit", "-qam", "mine"]);
    git(e.repo.path(), &["fetch", "-q", "origin"]);
    let mut rt = screen(&e);
    click(&mut rt, "pull-merge");
    step(&mut rt);
    click(&mut rt, "pull-run");
    settle(&mut rt, |s| s.conflicts.open && s.operation_kind.is_some());
    assert!(shows(&rt, "Résolution · fusion"));
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("conflits"))
    );
}

#[test]
fn a_failing_remote_changes_nothing_locally_and_says_so() {
    let e = env();
    std::fs::write(e.repo.path().join("a.txt"), "unsaved edit\n").unwrap();
    let before = git(e.repo.path(), &["status", "--porcelain=v2", "--branch"]);
    git(
        e.repo.path(),
        &[
            "remote",
            "set-url",
            "origin",
            "file:///nonexistent/for/sure.git",
        ],
    );
    let mut rt = screen(&e);
    click(&mut rt, "pull-run");
    settle(&mut rt, finished);
    assert!(shows(
        &rt,
        "rien n'a été intégré et votre travail local est intact"
    ));
    assert_eq!(
        git(e.repo.path(), &["status", "--porcelain=v2", "--branch"]),
        before
    );
    assert_eq!(
        std::fs::read_to_string(e.repo.path().join("a.txt")).unwrap(),
        "unsaved edit\n"
    );
}

#[test]
fn a_branch_without_a_tracked_upstream_explains_what_to_do() {
    let e = env();
    git(e.repo.path(), &["config", "--unset", "branch.main.remote"]);
    git(e.repo.path(), &["config", "--unset", "branch.main.merge"]);
    let mut rt = runtime(super::opened(e.repo.path()));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    rt.state.select_branch("main");
    for _ in 0..6 {
        step(&mut rt);
    }
    click(&mut rt, "branch-pull");
    settle(&mut rt, |s| {
        s.pulling.state.as_ref().is_some_and(|p| p.error.is_some())
    });
    assert!(shows(&rt, "ne suit aucune branche distante"));
    assert!(rect_of(&rt, "pull-run").is_none());
}
