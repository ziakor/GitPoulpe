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
    if key.starts_with("reset-") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("hist-panel", target));
        for _ in 0..8 {
            step(rt);
        }
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

struct Env {
    repo: tempfile::TempDir,
    c1: String,
    c3: String,
}

fn env() -> Env {
    let repo = tempfile::tempdir().unwrap();
    let p = repo.path();
    git(p, &["init", "-q", "-b", "main"]);
    for n in ["c1", "c2", "c3"] {
        std::fs::write(p.join("a.txt"), format!("{n}\n")).unwrap();
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", n]);
    }
    let c1 = git(p, &["rev-parse", "HEAD~2"]);
    let c3 = git(p, &["rev-parse", "HEAD"]);
    Env { repo, c1, c3 }
}

fn screen(e: &Env) -> Runtime<State> {
    let mut rt = runtime(super::opened(e.repo.path()));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "history");
    settle(&mut rt, |s| s.history.commits.len() >= 3);
    let index = rt
        .state
        .history
        .commits
        .iter()
        .position(|c| c.oid == e.c1)
        .unwrap();
    rt.state.select_commit(index);
    for _ in 0..6 {
        step(&mut rt);
    }
    click(&mut rt, "reset-here");
    settle(&mut rt, |s| {
        s.resetting
            .state
            .as_ref()
            .is_some_and(|r| r.preview.is_some())
    });
    rt
}

#[test]
fn each_mode_is_described_before_it_runs_and_the_previous_tip_is_announced() {
    let e = env();
    let mut rt = screen(&e);
    assert!(shows(&rt, "mixed : « main » passe sur ce commit"));
    assert!(shows(&rt, "2 commit(s) ne seront plus sur la branche"));
    click(&mut rt, "reset-soft");
    settle(&mut rt, |s| {
        s.resetting
            .state
            .as_ref()
            .is_some_and(|r| r.preview.is_some())
    });
    assert!(shows(&rt, "soft : « main » passe sur ce commit"));
    click(&mut rt, "reset-run");
    settle(&mut rt, |s| {
        s.resetting.state.as_ref().is_some_and(|r| r.done.is_some())
    });
    assert_eq!(git(e.repo.path(), &["rev-parse", "HEAD"]), e.c1);
    assert!(shows(&rt, "L'ancien sommet"));
    assert!(shows(&rt, "refs/gitpoulpe/recovery/"));
    assert_eq!(
        git(e.repo.path(), &["diff", "--cached", "--name-only"]),
        "a.txt"
    );
}

#[test]
fn a_hard_reset_that_would_erase_work_names_it_and_needs_a_second_explicit_click() {
    let e = env();
    std::fs::write(e.repo.path().join("a.txt"), "unsaved work\n").unwrap();
    let mut rt = screen(&e);
    click(&mut rt, "reset-hard");
    settle(&mut rt, |s| {
        s.resetting
            .state
            .as_ref()
            .is_some_and(|r| r.preview.as_ref().is_some_and(|p| !p.would_lose.is_empty()))
    });
    assert!(shows(
        &rt,
        "Modifications non enregistrées qui seraient effacées : a.txt"
    ));
    assert!(shows(&rt, "Limites"));
    click(&mut rt, "reset-run");
    settle(&mut rt, |s| {
        s.resetting.state.as_ref().is_some_and(|r| r.confirm)
    });
    assert_eq!(git(e.repo.path(), &["rev-parse", "HEAD"]), e.c3);
    assert_eq!(
        std::fs::read_to_string(e.repo.path().join("a.txt")).unwrap(),
        "unsaved work\n"
    );
    click(&mut rt, "reset-confirm-cancel");
    settle(&mut rt, |s| {
        s.resetting.state.as_ref().is_some_and(|r| !r.confirm)
    });
    click(&mut rt, "reset-run");
    settle(&mut rt, |s| {
        s.resetting.state.as_ref().is_some_and(|r| r.confirm)
    });
    click(&mut rt, "reset-confirm");
    settle(&mut rt, |s| {
        s.resetting.state.as_ref().is_some_and(|r| r.done.is_some())
    });
    assert_eq!(git(e.repo.path(), &["rev-parse", "HEAD"]), e.c1);
    assert_eq!(
        std::fs::read_to_string(e.repo.path().join("a.txt")).unwrap(),
        "c1\n"
    );
    assert!(shows(&rt, "sauvegardées sous refs/gitpoulpe/recovery/"));
    let work = git(
        e.repo.path(),
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/gitpoulpe/recovery",
        ],
    );
    let work = work
        .lines()
        .find(|l| l.ends_with("-work"))
        .unwrap()
        .to_owned();
    assert_eq!(
        git(e.repo.path(), &["show", &format!("{work}:a.txt")]),
        "unsaved work"
    );
}
