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

fn click(rt: &mut Runtime<State>, key: &str) {
    if key.starts_with("seq-") || key.starts_with("pick-") || key.starts_with("revert-") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("hist-panel", target));
        for _ in 0..8 {
            step(rt);
        }
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn open_history(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "history");
    settle(&mut rt, |s| s.history.commits.len() >= 4);
    rt
}

fn pick_commit(rt: &mut Runtime<State>, rev: &str, dir: &Path) {
    let oid = git(dir, &["rev-parse", rev]);
    let index = rt
        .state
        .history
        .commits
        .iter()
        .position(|c| c.oid == oid)
        .unwrap();
    rt.state.select_commit(index);
    for _ in 0..6 {
        step(rt);
    }
}

fn side_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("base.txt"), "base\n").unwrap();
    commit_all(p, "base");
    git(p, &["checkout", "-q", "-b", "side"]);
    for n in ["one", "two", "three"] {
        std::fs::write(p.join(format!("{n}.txt")), n).unwrap();
        commit_all(p, &format!("side {n}"));
    }
    git(p, &["checkout", "-q", "main"]);
    dir
}

#[test]
fn commits_are_queued_reordered_and_applied_in_the_announced_order() {
    let dir = side_repo();
    let mut rt = open_history(dir.path());
    pick_commit(&mut rt, "side~2", dir.path());
    click(&mut rt, "pick-add");
    settle(&mut rt, |s| {
        s.sequencing
            .state
            .as_ref()
            .is_some_and(|q| q.preview.is_some())
    });
    pick_commit(&mut rt, "side", dir.path());
    click(&mut rt, "pick-add");
    settle(&mut rt, |s| {
        s.sequencing
            .state
            .as_ref()
            .is_some_and(|q| q.queue.len() == 2 && q.preview.is_some())
    });
    assert!(shows(&rt, "1. "));
    assert!(shows(&rt, "side one"));
    click(&mut rt, "seq-up1");
    settle(&mut rt, |s| {
        s.sequencing
            .state
            .as_ref()
            .is_some_and(|q| q.preview.is_some())
    });
    assert!(
        shows(&rt, "1. ") && rt.state.sequencing.state.as_ref().unwrap().queue[0].1 == "side three"
    );
    click(&mut rt, "seq-run");
    settle(&mut rt, |s| {
        s.sequencing.state.is_none() && s.notice.as_deref().is_some_and(|n| n.contains("terminé"))
    });
    let log = git(dir.path(), &["log", "--format=%s", "-2"]);
    assert_eq!(log.lines().collect::<Vec<_>>(), ["side one", "side three"]);
}

#[test]
fn a_conflict_in_the_sequence_opens_the_resolution_where_the_commit_can_be_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "base\n").unwrap();
    commit_all(p, "base");
    git(p, &["checkout", "-q", "-b", "side"]);
    std::fs::write(p.join("a.txt"), "side\n").unwrap();
    commit_all(p, "side edit");
    git(p, &["checkout", "-q", "main"]);
    std::fs::write(p.join("a.txt"), "main\n").unwrap();
    commit_all(p, "main edit");
    let head = git(p, &["rev-parse", "HEAD"]);
    let mut rt = runtime(super::opened(p));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "history");
    settle(&mut rt, |s| s.history.commits.len() >= 3);
    pick_commit(&mut rt, "side", p);
    click(&mut rt, "pick-add");
    settle(&mut rt, |s| {
        s.sequencing
            .state
            .as_ref()
            .is_some_and(|q| q.preview.is_some())
    });
    click(&mut rt, "seq-run");
    settle(&mut rt, |s| s.conflicts.open && s.operation_kind.is_some());
    assert!(shows(&rt, "Résolution · cherry-pick"));
    assert!(shows(&rt, "le commit repris"));
    assert!(rect_of(&rt, "cf-skip").is_some());
    click(&mut rt, "cf-skip");
    settle(&mut rt, |s| !s.conflicts.open && s.operation_kind.is_none());
    assert_eq!(git(p, &["rev-parse", "HEAD"]), head);
    assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), "main\n");
}

#[test]
fn reverting_a_merge_commit_requires_choosing_its_parent() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "base\n").unwrap();
    commit_all(p, "base");
    git(p, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(p.join("f.txt"), "f\n").unwrap();
    commit_all(p, "feature work");
    git(p, &["checkout", "-q", "main"]);
    std::fs::write(p.join("m.txt"), "m\n").unwrap();
    commit_all(p, "main work");
    git(
        p,
        &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"],
    );
    let mut rt = runtime(super::opened(p));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "history");
    settle(&mut rt, |s| s.history.commits.len() >= 4);
    pick_commit(&mut rt, "main", p);
    click(&mut rt, "revert-add");
    settle(&mut rt, |s| {
        s.sequencing
            .state
            .as_ref()
            .is_some_and(|q| q.preview.is_some())
    });
    assert!(shows(&rt, "choisissez explicitement le parent"));
    click(&mut rt, "seq-run");
    settle(&mut rt, |s| {
        s.sequencing
            .state
            .as_ref()
            .is_some_and(|q| q.error.is_some() && !q.busy)
    });
    assert!(shows(&rt, "choisissez explicitement le parent à garder"));
    assert!(p.join("f.txt").exists());
    click(&mut rt, "seq-parent1");
    click(&mut rt, "seq-run");
    settle(&mut rt, |s| s.sequencing.state.is_none());
    assert!(!p.join("f.txt").exists());
    assert!(git(p, &["log", "-1", "--format=%s"]).starts_with("Revert \"merge feature\""));
}
