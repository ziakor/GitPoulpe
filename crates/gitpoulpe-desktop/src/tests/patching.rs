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
    assert!(output.status.success(), "{args:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
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

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    dir
}

fn patch_file(dir: &Path, name: &str, edit: &str) -> std::path::PathBuf {
    let other = tempfile::tempdir().unwrap();
    git(other.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(other.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(other.path(), &["add", "-A"]);
    git(other.path(), &["commit", "-q", "-m", "init"]);
    std::fs::write(other.path().join("a.txt"), edit).unwrap();
    let diff = git(other.path(), &["diff"]) + "\n";
    let path = dir.join(name);
    std::fs::write(&path, diff).unwrap();
    path
}

fn screen(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    for _ in 0..20 {
        step(&mut rt);
    }
    rt.state.open_patch_import();
    step(&mut rt);
    rt
}

fn load(rt: &mut Runtime<State>, path: std::path::PathBuf) {
    rt.state.patch_load(path);
    settle(rt, |s| !s.patches.busy);
}

#[test]
fn the_patch_is_inspected_before_anything_is_applied() {
    let dir = repo();
    let scratch = tempfile::tempdir().unwrap();
    let patch = patch_file(scratch.path(), "p.patch", "one\nTWO\nthree\nfour\n");
    let mut rt = screen(dir.path());
    assert!(shows(&rt, "Importer un patch"));
    load(&mut rt, patch);
    assert!(shows(&rt, "1 fichier ciblé"));
    assert!(shows(&rt, "a.txt"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one\ntwo\nthree\n"
    );
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "");
}

#[test]
fn applying_requires_a_successful_check_and_then_changes_the_chosen_targets() {
    let dir = repo();
    let scratch = tempfile::tempdir().unwrap();
    let patch = patch_file(scratch.path(), "p.patch", "one\nTWO\nthree\n");
    let mut rt = screen(dir.path());
    load(&mut rt, patch);
    click(&mut rt, "pt-apply");
    step(&mut rt);
    assert!(shows(&rt, "Vérifiez d'abord"));
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "");
    click(&mut rt, "pt-mode-index");
    step(&mut rt);
    click(&mut rt, "pt-check");
    settle(&mut rt, |s| !s.patches.busy && s.patches.checked.is_some());
    assert!(shows(&rt, "s'applique entièrement"));
    click(&mut rt, "pt-apply");
    settle(&mut rt, |s| !s.patches.busy && s.patches.notice.is_some());
    assert!(shows(&rt, "index seulement"));
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "MM a.txt");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one\ntwo\nthree\n"
    );
}

#[test]
fn a_patch_that_does_not_fit_is_refused_without_touching_the_files() {
    let dir = repo();
    let scratch = tempfile::tempdir().unwrap();
    let patch = patch_file(scratch.path(), "p.patch", "one\nTWO\nthree\n");
    std::fs::write(dir.path().join("a.txt"), "changed\nlocally\n").unwrap();
    let mut rt = screen(dir.path());
    load(&mut rt, patch);
    click(&mut rt, "pt-check");
    settle(&mut rt, |s| !s.patches.busy && s.patches.checked.is_some());
    assert!(matches!(rt.state.patches.checked, Some(Err(_))));
    assert!(rt.state.patches.error.is_some());
    click(&mut rt, "pt-apply");
    step(&mut rt);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "changed\nlocally\n"
    );
}

#[test]
fn unsafe_paths_and_missing_files_are_reported() {
    let dir = repo();
    let scratch = tempfile::tempdir().unwrap();
    let evil = scratch.path().join("evil.patch");
    std::fs::write(
        &evil,
        "diff --git a/../x b/../x\n--- a/../x\n+++ b/../x\n@@ -1 +1 @@\n-a\n+b\n",
    )
    .unwrap();
    let mut rt = screen(dir.path());
    load(&mut rt, evil);
    assert!(rt.state.patches.preview.is_none());
    assert!(rt.state.patches.error.is_some());
    rt.state.patch_apply();
    assert!(!rt.state.patches.busy);
    rt.state.patch_load(scratch.path().join("absent.patch"));
    assert!(shows(&rt, "lecture impossible") || rt.state.patches.error.is_some());
}

#[test]
fn three_way_conflicts_are_left_in_the_files_and_announced() {
    let dir = repo();
    let scratch = tempfile::tempdir().unwrap();
    let patch = patch_file(scratch.path(), "p.patch", "one\nTWO\nthree\n");
    std::fs::write(dir.path().join("a.txt"), "one\nOTHER\nthree\n").unwrap();
    git(dir.path(), &["commit", "-qam", "diverge"]);
    let mut rt = screen(dir.path());
    load(&mut rt, patch);
    click(&mut rt, "pt-3way");
    step(&mut rt);
    assert!(rt.state.patches.three_way);
    click(&mut rt, "pt-apply");
    settle(&mut rt, |s| {
        !s.patches.busy && (s.patches.notice.is_some() || s.patches.error.is_some())
    });
    assert!(shows(&rt, "conflits"), "{:?}", rt.state.patches.error);
    let content = std::fs::read_to_string(dir.path().join("a.txt")).unwrap();
    assert!(content.contains("<<<<<<<"));
}

#[test]
fn a_late_answer_and_closing_discard_stale_work() {
    let dir = repo();
    let mut rt = screen(dir.path());
    let stale = rt.state.patches.request;
    rt.state.close_patch_import();
    assert!(!rt.state.apply_patch_checked(stale, Ok(())));
    rt.state.open_patch_import();
    assert!(!rt.state.apply_patch_checked(stale, Ok(())));
}

fn exporter(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    for _ in 0..20 {
        step(&mut rt);
    }
    rt.state.open_patch_export();
    step(&mut rt);
    rt
}

fn destination(rt: &mut Runtime<State>, path: &Path) {
    rt.state.patches.destination = rune::TextEdit::new(path.to_str().unwrap());
    step(rt);
}

#[test]
fn exporting_writes_exactly_the_chosen_source_to_the_chosen_file() {
    let dir = repo();
    std::fs::write(dir.path().join("a.txt"), "one\nTWO\nthree\n").unwrap();
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("changes.patch");
    let mut rt = exporter(dir.path());
    assert!(shows(&rt, "Exporter en patch"));
    destination(&mut rt, &file);
    click(&mut rt, "pt-src-index");
    step(&mut rt);
    click(&mut rt, "pt-export");
    settle(&mut rt, |s| !s.patches.busy && s.patches.error.is_some());
    assert!(shows(&rt, "rien à exporter"));
    assert!(!file.exists());
    click(&mut rt, "pt-src-tree");
    step(&mut rt);
    click(&mut rt, "pt-export");
    settle(&mut rt, |s| !s.patches.busy && s.patches.notice.is_some());
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.contains("+TWO") && written.contains("a.txt"));
    assert!(shows(&rt, "Patch exporté"));
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "M a.txt");
}

#[test]
fn commits_are_exported_and_the_file_reapplies_on_a_compatible_base() {
    let dir = repo();
    std::fs::write(dir.path().join("a.txt"), "one\nTWO\nthree\n").unwrap();
    git(dir.path(), &["commit", "-qam", "change two"]);
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("c.patch");
    let mut rt = exporter(dir.path());
    destination(&mut rt, &file);
    click(&mut rt, "pt-src-commits");
    step(&mut rt);
    assert!(rect_of(&rt, "pt-revision").is_some());
    click(&mut rt, "pt-export");
    settle(&mut rt, |s| !s.patches.busy && s.patches.notice.is_some());
    let other = repo();
    let mut import = screen(other.path());
    load(&mut import, file);
    assert!(shows(&import, "1 fichier ciblé"));
    click(&mut import, "pt-check");
    settle(&mut import, |s| {
        !s.patches.busy && s.patches.checked.is_some()
    });
    assert!(matches!(import.state.patches.checked, Some(Ok(()))));
}

#[test]
fn an_unusable_destination_reports_the_failure_and_leaves_no_file() {
    let dir = repo();
    std::fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
    let out = tempfile::tempdir().unwrap();
    let existing = out.path().join("keep.patch");
    std::fs::write(&existing, "precious").unwrap();
    let mut rt = exporter(dir.path());
    click(&mut rt, "pt-export");
    step(&mut rt);
    assert!(shows(&rt, "Indiquez le fichier de destination"));
    destination(&mut rt, &out.path().join("missing/x.patch"));
    click(&mut rt, "pt-export");
    settle(&mut rt, |s| !s.patches.busy && s.patches.error.is_some());
    assert!(shows(&rt, "destination inutilisable"));
    assert!(!rt.state.patches.notice.is_some());
    destination(&mut rt, &existing);
    click(&mut rt, "pt-export");
    settle(&mut rt, |s| !s.patches.busy && s.patches.error.is_some());
    assert_eq!(std::fs::read_to_string(&existing).unwrap(), "precious");
    click(&mut rt, "pt-overwrite");
    step(&mut rt);
    click(&mut rt, "pt-export");
    settle(&mut rt, |s| !s.patches.busy && s.patches.notice.is_some());
    assert!(
        std::fs::read_to_string(&existing)
            .unwrap()
            .contains("changed")
    );
}
