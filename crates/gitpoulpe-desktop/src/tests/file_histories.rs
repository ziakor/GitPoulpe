use std::path::Path;
use std::process::Command;

use rune::Runtime;

use super::{click_at, rect_of, runtime, step};
use crate::state::State;

fn git(dir: &Path, args: &[&str]) {
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
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn settle(rt: &mut Runtime<State>, until: impl Fn(&State) -> bool) {
    for _ in 0..300 {
        step(rt);
        if until(&rt.state) {
            for _ in 0..8 {
                step(rt);
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the screen never settled");
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn lineage() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let commit = |message: &str| {
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", message]);
    };
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
    commit("add a");
    git(p, &["mv", "a.txt", "b.txt"]);
    commit("rename to b");
    std::fs::write(p.join("b.txt"), "one\ntwo\nthree\nfour\nfive\nsix\n").unwrap();
    commit("edit b");
    git(p, &["mv", "b.txt", "dir_c.txt"]);
    commit("rename to c");
    git(p, &["rm", "-q", "dir_c.txt"]);
    commit("delete c");
    std::fs::write(p.join("dir_c.txt"), "brand new\n").unwrap();
    commit("recreate c");
    std::fs::write(p.join("dir_c.txt"), "brand new\nlocal edit\n").unwrap();
    dir
}

fn history_screen(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    for _ in 0..40 {
        step(&mut rt);
        if rect_of(&rt, "fh0").is_some() {
            break;
        }
    }
    click(&mut rt, "fh0");
    settle(&mut rt, |s| s.file_history.open && !s.file_history.loading);
    rt
}

fn open_revision(rt: &mut Runtime<State>, subject: &str) -> String {
    let oid = rt
        .state
        .file_history
        .history
        .as_ref()
        .unwrap()
        .revisions
        .iter()
        .find(|r| r.subject == subject)
        .unwrap()
        .oid
        .clone();
    rt.state.select_file_revision(oid.clone());
    settle(rt, |s| !s.file_history.revision_loading);
    oid
}

#[test]
fn the_history_lists_every_step_and_marks_a_change_of_path() {
    let dir = lineage();
    let rt = history_screen(dir.path());
    assert!(shows(&rt, "Historique de dir_c.txt · 6 révision(s)"));
    assert!(shows(&rt, "recreate c") && shows(&rt, "delete c") && shows(&rt, "rename to c"));
    assert!(shows(&rt, "renommé depuis b.txt"));
    assert!(shows(&rt, "chemin : b.txt"));
    assert!(shows(&rt, "supprimé"));
}

#[test]
fn a_selected_revision_shows_its_diff_and_content_and_survives_a_refresh() {
    let dir = lineage();
    let mut rt = history_screen(dir.path());
    let oid = open_revision(&mut rt, "edit b");
    assert!(shows(&rt, "+six"));
    click(&mut rt, "fh-show-content");
    step(&mut rt);
    assert!(shows(&rt, "five"));
    click(&mut rt, "fh-refresh");
    settle(&mut rt, |s| !s.file_history.loading);
    assert_eq!(
        rt.state.file_history.selected.as_deref(),
        Some(oid.as_str())
    );
    assert!(rt.state.file_history.notice.is_none());
    assert!(rt.state.file_history.revision.is_some());
}

#[test]
fn a_deleted_revision_has_an_explicit_state_and_keeps_the_selection_when_moving_on() {
    let dir = lineage();
    let mut rt = history_screen(dir.path());
    let deleted = open_revision(&mut rt, "delete c");
    click(&mut rt, "fh-show-content");
    step(&mut rt);
    assert!(shows(&rt, "supprimé à cette révision"));
    assert!(rt.state.file_history.revision.as_ref().unwrap().deleted);
    let before = rt.state.file_history.revision_request;
    assert!(
        !rt.state
            .apply_file_revision(before - 1, Err("late".to_owned()))
    );
    assert_eq!(
        rt.state.file_history.selected.as_deref(),
        Some(deleted.as_str())
    );
    assert!(rt.state.file_history.revision_error.is_none());
}

#[test]
fn a_shallow_clone_says_its_history_is_incomplete() {
    let origin = tempfile::tempdir().unwrap();
    let p = origin.path();
    git(p, &["init", "-q", "-b", "main"]);
    for n in 0..4 {
        std::fs::write(p.join("f.txt"), format!("version {n}\n")).unwrap();
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", &format!("change {n}")]);
    }
    let clone = tempfile::tempdir().unwrap();
    git(
        clone.path(),
        &[
            "clone",
            "-q",
            "--depth",
            "2",
            &format!("file://{}", p.display()),
            ".",
        ],
    );
    std::fs::write(clone.path().join("f.txt"), "local\n").unwrap();
    let rt = history_screen(clone.path());
    assert!(shows(&rt, "Historique incomplet"));
    assert!(shows(&rt, "2 révision(s)"));
}

#[test]
fn escape_returns_to_the_files() {
    let dir = lineage();
    let mut rt = history_screen(dir.path());
    super::press(&mut rt, rune::Key::Escape);
    step(&mut rt);
    assert!(!rt.state.file_history.open);
}

#[test]
fn a_selection_that_left_the_history_is_dropped_with_a_visible_reason() {
    let dir = lineage();
    let mut rt = history_screen(dir.path());
    let oid = open_revision(&mut rt, "edit b");
    rt.state.reload_file_history();
    let mut shorter = rt.state.file_history.history.clone().unwrap();
    shorter.revisions.retain(|r| r.oid != oid);
    let request = rt.state.file_history.request;
    assert!(rt.state.apply_file_history(request, Ok(shorter)));
    step(&mut rt);
    assert!(rt.state.file_history.selected.is_none() && rt.state.file_history.revision.is_none());
    assert!(shows(&rt, "n'est plus dans l'historique de ce fichier"));
}

fn blame_ready(rt: &mut Runtime<State>) {
    settle(rt, |s| {
        !s.file_history.blame_loading
            && (s.file_history.blame.is_some() || s.file_history.blame_error.is_some())
    });
}

#[test]
fn the_local_file_blame_tells_unsaved_lines_from_historical_ones() {
    let dir = lineage();
    let mut rt = history_screen(dir.path());
    click(&mut rt, "fh-blame-local");
    blame_ready(&mut rt);
    assert!(shows(
        &rt,
        "Blame de dir_c.txt · fichier local (copie de travail) · 2 ligne(s) dont 1 non enregistrée(s)"
    ));
    let blame = rt.state.file_history.blame.as_ref().unwrap();
    assert!(blame.lines[0].oid.is_some() && blame.lines[1].oid.is_none());
    assert_eq!(blame.lines[1].text, "local edit");
    assert!(shows(&rt, "non enregistré"));
}

#[test]
fn the_blame_of_a_revision_follows_the_selection_and_opens_the_attributed_commit() {
    let dir = lineage();
    let mut rt = history_screen(dir.path());
    let edit = open_revision(&mut rt, "edit b");
    click(&mut rt, "fh-show-blame");
    blame_ready(&mut rt);
    let blame = rt.state.file_history.blame.as_ref().unwrap().clone();
    assert_eq!(blame.revision.as_deref(), Some(edit.as_str()));
    assert_eq!(blame.path, "b.txt");
    assert_eq!(blame.lines.len(), 6);
    assert_eq!(blame.lines[5].oid.as_deref(), Some(edit.as_str()));
    assert!(shows(&rt, "révision"));
    let origin = blame.lines[0].oid.clone().unwrap();
    click(&mut rt, "fbk0");
    settle(&mut rt, |s| !s.file_history.revision_loading);
    assert_eq!(
        rt.state.file_history.mode,
        crate::file_history::PanelMode::Diff
    );
    assert_eq!(
        rt.state.file_history.selected.as_deref(),
        Some(origin.as_str())
    );
    assert_eq!(
        rt.state.file_history.revision.as_ref().unwrap().path,
        "a.txt"
    );
}

#[test]
fn an_attributed_commit_missing_from_the_loaded_history_is_reported_not_guessed() {
    let dir = lineage();
    let mut rt = history_screen(dir.path());
    click(&mut rt, "fh-blame-local");
    blame_ready(&mut rt);
    let before = rt.state.file_history.selected.clone();
    rt.state.blame_open_commit("d".repeat(40));
    step(&mut rt);
    assert_eq!(rt.state.file_history.selected, before);
    assert!(shows(&rt, "n'est pas dans la liste chargée"));
    let request = rt.state.file_history.blame_request;
    assert!(
        !rt.state
            .apply_file_blame(request - 1, Err("late".to_owned()))
    );
}

#[test]
fn a_large_file_is_attributed_without_mounting_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    let body: String = (0..5000).map(|n| format!("line {n}\n")).collect();
    std::fs::write(p.join("big.txt"), &body).unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "big"]);
    std::fs::write(
        p.join("big.txt"),
        body.replacen("line 10\n", "line 10 local\n", 1),
    )
    .unwrap();
    let mut rt = history_screen(p);
    click(&mut rt, "fh-blame-local");
    blame_ready(&mut rt);
    assert_eq!(
        rt.state.file_history.blame.as_ref().unwrap().lines.len(),
        5000
    );
    assert!(super::mounted(&rt, "fb-row") > 0 && super::mounted(&rt, "fb-row") < 200);
}

#[test]
fn choosing_another_revision_while_reading_the_blame_attributes_that_revision() {
    let dir = lineage();
    let mut rt = history_screen(dir.path());
    open_revision(&mut rt, "edit b");
    click(&mut rt, "fh-show-blame");
    blame_ready(&mut rt);
    let other = open_revision(&mut rt, "rename to b");
    settle(&mut rt, |s| {
        s.file_history
            .blame
            .as_ref()
            .is_some_and(|b| b.revision.as_deref() == Some(other.as_str()))
    });
    let blame = rt.state.file_history.blame.as_ref().unwrap();
    assert_eq!(blame.lines.len(), 5);
}
