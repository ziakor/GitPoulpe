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
    if key.starts_with("rl-") && !key.starts_with("rl-row") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("rl-panel", target));
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
    for n in ["one", "two", "three"] {
        std::fs::write(p.join("a.txt"), format!("{n}\n")).unwrap();
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", n]);
    }
    git(p, &["reset", "-q", "--hard", "HEAD~2"]);
    dir
}

fn screen(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    click(&mut rt, "branches-reflog");
    settle(&mut rt, |s| s.reflog.open && !s.reflog.loading);
    rt
}

#[test]
fn the_entries_show_operation_date_and_commit_for_the_head_and_the_branch() {
    let dir = repo();
    let mut rt = screen(dir.path());
    assert!(rt.state.reflog.entries.len() >= 4);
    assert!(shows(&rt, "HEAD@{0} · reset: moving to HEAD~2"));
    assert!(shows(&rt, "commit: three"));
    click(&mut rt, "rl-scope-branch");
    settle(&mut rt, |s| {
        s.reflog.branch_scope && !s.reflog.loading && !s.reflog.entries.is_empty()
    });
    assert!(shows(&rt, "main@{0}"));
}

#[test]
fn an_old_commit_is_recovered_in_a_new_branch_without_moving_the_current_one() {
    let dir = repo();
    let three = git(dir.path(), &["rev-parse", "HEAD@{1}"]);
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    let mut rt = screen(dir.path());
    let index = rt
        .state
        .reflog
        .entries
        .iter()
        .position(|e| e.message.contains("commit: three"))
        .unwrap();
    rt.state.select_reflog(index);
    settle(&mut rt, |s| s.reflog.inspected.is_some());
    assert!(shows(&rt, "Aucune branche ni tag n'atteint plus ce commit"));
    rt.state.reflog.name = TextEdit::new("recup-three");
    step(&mut rt);
    click(&mut rt, "rl-recover");
    settle(&mut rt, |s| {
        !s.reflog.busy
            && s.notice
                .as_deref()
                .is_some_and(|n| n.contains("recup-three"))
    });
    assert_eq!(
        git(dir.path(), &["rev-parse", "refs/heads/recup-three"]),
        three
    );
    assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(dir.path(), &["symbolic-ref", "--short", "HEAD"]),
        "main"
    );
}

#[test]
fn an_entry_whose_object_is_missing_cannot_be_inspected_or_recovered() {
    let dir = repo();
    let log = dir.path().join(".git/logs/HEAD");
    let mut text = std::fs::read_to_string(&log).unwrap();
    let tip = git(dir.path(), &["rev-parse", "HEAD"]);
    text.push_str(&format!(
        "{tip} {} t <t@example.com> 1700000000 +0000\tcommit: ghost\n",
        "cd".repeat(20)
    ));
    std::fs::write(&log, text).unwrap();
    let mut rt = screen(dir.path());
    assert!(shows(&rt, "objet absent"));
    rt.state.select_reflog(0);
    settle(&mut rt, |s| s.reflog.inspected.is_some());
    assert!(shows(&rt, "n'est plus dans le dépôt"));
    assert!(rect_of(&rt, "rl-recover").is_none());
    rt.state.reflog.name = TextEdit::new("ghosty");
    rt.state.reflog_recover();
    settle(&mut rt, |s| {
        !s.reflog.busy
            && s.notice
                .as_deref()
                .is_some_and(|n| n.contains("impossible"))
    });
    assert!(git(dir.path(), &["for-each-ref", "refs/heads/ghosty"]).is_empty());
}
