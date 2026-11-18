use std::path::Path;
use std::process::Command;

use rune::{Runtime, TextEdit};

use super::{click_at, press, rect_of, runtime, step};
use crate::state::State;

fn git_out(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Ada")
        .env("GIT_AUTHOR_EMAIL", "ada@example.org")
        .env("GIT_COMMITTER_NAME", "Ada")
        .env("GIT_COMMITTER_EMAIL", "ada@example.org")
        .output()
        .unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = git_out(dir, args);
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
    let c = &rt.state.conflicts;
    panic!(
        "the screen never settled: busy={} loading={} files={:?} states={:?} sel={:?} notice={:?}",
        c.busy,
        c.loading,
        c.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>(),
        c.states,
        c.selected,
        rt.state.notice
    );
}

fn idle(s: &State) -> bool {
    !s.conflicts.busy && !s.conflicts.loading
}

fn click(rt: &mut Runtime<State>, key: &str) {
    if key.starts_with("cf-") && !key.starts_with("cf-row") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("cf-panel", target));
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
    std::fs::write(p.join("a.txt"), "line1\nline2\nline3\n").unwrap();
    std::fs::write(p.join("b.txt"), "b\n").unwrap();
    std::fs::write(p.join("gone.txt"), "g\n").unwrap();
    commit_all(p, "base");
    git(p, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(p.join("a.txt"), "line1\nfeature\nline3\n").unwrap();
    std::fs::write(p.join("b.txt"), "b feature\n").unwrap();
    std::fs::write(p.join("gone.txt"), "g changed\n").unwrap();
    commit_all(p, "feature work");
    git(p, &["checkout", "-q", "main"]);
    std::fs::write(p.join("a.txt"), "line1\nmain\nline3\n").unwrap();
    std::fs::write(p.join("b.txt"), "b main\n").unwrap();
    git(p, &["rm", "-q", "gone.txt"]);
    commit_all(p, "main work");
    assert!(!git_out(p, &["merge", "feature"]).status.success());
    dir
}

fn screen(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    settle(&mut rt, |s| s.operation_kind.is_some());
    click(&mut rt, "conflicts-open");
    settle(&mut rt, |s| {
        s.conflicts.open && idle(s) && s.conflicts.versions.is_some()
    });
    rt
}

fn pick(rt: &mut Runtime<State>, name: &str) {
    rt.state.select_conflict(name.into());
    settle(rt, |s| idle(s) && s.conflicts.versions.is_some());
}

#[test]
fn an_active_merge_is_announced_and_its_conflicts_are_listed_with_their_kind() {
    let dir = repo();
    let mut rt = runtime(super::opened(dir.path()));
    settle(&mut rt, |s| s.operation_kind.is_some());
    assert!(shows(&rt, "Fusion en cours · 3 fichier(s) en conflit"));
    click(&mut rt, "conflicts-open");
    settle(&mut rt, |s| s.conflicts.open && idle(s));
    let paths: Vec<String> = rt
        .state
        .conflicts
        .files
        .iter()
        .map(|f| f.path.display().to_string())
        .collect();
    assert_eq!(paths, ["a.txt", "b.txt", "gone.txt"]);
    assert!(shows(&rt, "supprimé de votre côté, modifié de l'autre"));
    assert!(shows(&rt, "Résolution · fusion"));
}

#[test]
fn the_versions_are_named_after_the_branches_and_a_side_can_be_taken() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "a.txt");
    assert!(shows(&rt, "votre branche main"));
    assert!(shows(&rt, "la branche fusionnée feature"));
    assert!(shows(&rt, "Version commune (base)"));
    click(&mut rt, "cf-take-theirs");
    settle(&mut rt, |s| idle(s) && s.conflicts.files.len() == 2);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "line1\nfeature\nline3\n"
    );
    assert!(shows(&rt, "Il reste 2 fichier(s)"));
    assert!(rect_of(&rt, "cf-continue").is_none() && rect_of(&rt, "cf-continue-off").is_some());
}

#[test]
fn an_edited_result_with_markers_needs_an_explicit_confirmation() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "a.txt");
    assert!(rt.state.conflicts.result.text.contains("<<<<<<<"));
    click(&mut rt, "cf-validate");
    settle(&mut rt, |s| idle(s) && s.conflicts.markers_pending);
    assert!(shows(&rt, "contient encore des marqueurs"));
    assert_eq!(rt.state.conflicts.files.len(), 3);
    rt.state.conflicts.result = TextEdit::new("line1\nmerged by hand\nline3\n");
    step(&mut rt);
    click(&mut rt, "cf-validate");
    settle(&mut rt, |s| idle(s) && s.conflicts.files.len() == 2);
    assert_eq!(
        git(dir.path(), &["show", ":a.txt"]),
        "line1\nmerged by hand\nline3"
    );
}

#[test]
fn a_deleted_by_one_side_file_offers_to_accept_the_deletion() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "gone.txt");
    assert!(shows(&rt, "Accepter la suppression"));
    click(&mut rt, "cf-take-ours");
    settle(&mut rt, |s| idle(s) && s.conflicts.files.len() == 2);
    assert!(!dir.path().join("gone.txt").exists());
}

#[test]
fn a_file_changed_after_its_resolution_blocks_continuing_until_validated_again() {
    let dir = repo();
    let mut rt = screen(dir.path());
    for name in ["a.txt", "b.txt", "gone.txt"] {
        pick(&mut rt, name);
        click(&mut rt, "cf-take-theirs");
        settle(&mut rt, |s| {
            idle(s)
                && s.conflicts.states.iter().any(|(p, st)| {
                    p == Path::new(name) && *st == gitpoulpe_core::Resolution::Resolved
                })
        });
    }
    settle(&mut rt, |s| s.conflicts.ready());
    assert!(rect_of(&rt, "cf-continue").is_some());
    std::fs::write(dir.path().join("a.txt"), "edited behind the screen\n").unwrap();
    rt.state.reload_conflicts();
    settle(&mut rt, |s| idle(s) && !s.conflicts.ready());
    assert!(shows(&rt, "à valider de nouveau"));
    assert!(rect_of(&rt, "cf-continue").is_none());
    rt.state.select_conflict("a.txt".into());
    step(&mut rt);
    assert!(shows(&rt, "modifié après sa résolution"));
    click(&mut rt, "cf-revalidate");
    settle(&mut rt, |s| idle(s) && s.conflicts.ready());
    assert!(rect_of(&rt, "cf-continue").is_some());
}

#[test]
fn continuing_creates_the_merge_commit_and_closes_the_screen() {
    let dir = repo();
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    let mut rt = screen(dir.path());
    for name in ["a.txt", "b.txt", "gone.txt"] {
        pick(&mut rt, name);
        click(&mut rt, "cf-take-theirs");
        settle(&mut rt, |s| {
            idle(s)
                && s.conflicts.states.iter().any(|(p, st)| {
                    p == Path::new(name) && *st == gitpoulpe_core::Resolution::Resolved
                })
        });
    }
    settle(&mut rt, |s| s.conflicts.ready());
    click(&mut rt, "cf-continue");
    settle(&mut rt, |s| !s.conflicts.open && s.operation_kind.is_none());
    let parents = git(dir.path(), &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(parents.split(' ').count(), 3);
    assert!(parents.contains(&head));
    assert!(!shows(&rt, "Fusion en cours"));
}

#[test]
fn abandoning_asks_first_restores_the_state_and_tells_where_the_work_was_kept() {
    let dir = repo();
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    std::fs::write(dir.path().join("mine.txt"), "untracked\n").unwrap();
    let mut rt = screen(dir.path());
    pick(&mut rt, "a.txt");
    rt.state.conflicts.result = TextEdit::new("line1\nmy resolution\nline3\n");
    step(&mut rt);
    click(&mut rt, "cf-validate");
    settle(&mut rt, |s| idle(s) && s.conflicts.files.len() == 2);
    click(&mut rt, "cf-abort");
    settle(&mut rt, |s| s.conflicts.confirm_abort);
    assert!(shows(&rt, "gitpoulpe-backup"));
    press(&mut rt, rune::Key::Escape);
    assert!(!rt.state.conflicts.confirm_abort && rt.state.conflicts.open);
    assert!(
        git_out(dir.path(), &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .status
            .success()
    );
    click(&mut rt, "cf-abort");
    settle(&mut rt, |s| s.conflicts.confirm_abort);
    click(&mut rt, "cf-abort-confirm-yes");
    settle(&mut rt, |s| !s.conflicts.open && s.operation_kind.is_none());
    assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("mine.txt")).unwrap(),
        "untracked\n"
    );
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("sont conservés dans"))
    );
    let backup = std::fs::read_dir(dir.path().join(".git/gitpoulpe-backup"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        std::fs::read_to_string(backup.join("a.txt")).unwrap(),
        "line1\nmy resolution\nline3\n"
    );
}

#[test]
fn abandoning_without_the_confirmation_does_nothing() {
    let dir = repo();
    let mut rt = screen(dir.path());
    rt.state.conflict_abort();
    for _ in 0..10 {
        step(&mut rt);
    }
    assert!(!rt.state.conflicts.busy);
    assert!(
        git_out(dir.path(), &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .status
            .success()
    );
}
