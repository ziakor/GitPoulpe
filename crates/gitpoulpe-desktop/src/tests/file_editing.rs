use std::os::unix::fs::PermissionsExt;
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
    std::fs::write(p.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(p.join("b.txt"), "other\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    std::fs::write(p.join("a.txt"), "one\ntwo\nlocal\n").unwrap();
    dir
}

fn editor(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    for _ in 0..40 {
        step(&mut rt);
        if rect_of(&rt, "ed0").is_some() {
            break;
        }
    }
    click(&mut rt, "ed0");
    settle(&mut rt, |s| s.file_edit.open && !s.file_edit.loading);
    rt
}

fn type_text(rt: &mut Runtime<State>, text: &str) {
    rt.state.file_edit.text = TextEdit::new(text);
    step(rt);
}

#[test]
fn saving_changes_only_the_announced_file() {
    let dir = repo();
    let mut rt = editor(dir.path());
    assert!(shows(&rt, "Édition de a.txt"));
    assert!(rect_of(&rt, "fe-editor").is_some());
    type_text(&mut rt, "one\ntwo\nlocal\nmore\n");
    assert!(shows(&rt, "modifications non enregistrées"));
    click(&mut rt, "fe-save");
    settle(&mut rt, |s| {
        !s.file_edit.saving && s.file_edit.notice.is_some()
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one\ntwo\nlocal\nmore\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "other\n"
    );
    assert!(shows(&rt, "a.txt enregistré."));
    assert!(!rt.state.file_edit.dirty());
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "M a.txt");
}

#[test]
fn an_external_change_offers_an_explicit_choice_before_anything_is_overwritten() {
    let dir = repo();
    let mut rt = editor(dir.path());
    type_text(&mut rt, "mine\n");
    std::fs::write(dir.path().join("a.txt"), "theirs\n").unwrap();
    click(&mut rt, "fe-save");
    settle(&mut rt, |s| {
        !s.file_edit.saving && s.file_edit.save_error.is_some()
    });
    assert!(rt.state.file_edit.external);
    assert!(shows(&rt, "a changé sur le disque"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "theirs\n"
    );
    click(&mut rt, "fe-keep");
    step(&mut rt);
    assert!(!rt.state.file_edit.external);
    assert_eq!(rt.state.file_edit.text.text, "mine\n");
    click(&mut rt, "fe-save");
    settle(&mut rt, |s| !s.file_edit.saving && s.file_edit.external);
    click(&mut rt, "fe-overwrite");
    settle(&mut rt, |s| {
        !s.file_edit.saving && s.file_edit.notice.is_some()
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "mine\n"
    );
}

#[test]
fn reloading_after_an_external_change_takes_the_disk_version_on_purpose() {
    let dir = repo();
    let mut rt = editor(dir.path());
    type_text(&mut rt, "mine\n");
    std::fs::write(dir.path().join("a.txt"), "theirs\n").unwrap();
    click(&mut rt, "fe-save");
    settle(&mut rt, |s| !s.file_edit.saving && s.file_edit.external);
    click(&mut rt, "fe-reload");
    settle(&mut rt, |s| !s.file_edit.loading);
    assert_eq!(rt.state.file_edit.text.text, "theirs\n");
    assert!(!rt.state.file_edit.dirty() && !rt.state.file_edit.external);
}

#[test]
fn binary_foreign_encoded_and_historical_content_are_never_opened_for_editing() {
    let dir = repo();
    let p = dir.path();
    std::fs::write(p.join("bin.dat"), [0u8, 1, 2]).unwrap();
    std::fs::write(p.join("latin1.txt"), b"caf\xE9\n").unwrap();
    let mut rt = editor(p);
    for (path, needle) in [
        ("bin.dat", "fichier binaire"),
        ("latin1.txt", "encodage non pris en charge"),
    ] {
        rt.state.open_file_edit(path.to_owned());
        settle(&mut rt, |s| !s.file_edit.loading);
        assert!(shows(&rt, "Édition impossible"), "{path}");
        assert!(shows(&rt, needle), "{path}");
        assert!(rect_of(&rt, "fe-editor").is_none());
        rt.state.file_edit_save(false);
        assert!(!rt.state.file_edit.saving);
    }
    assert_eq!(std::fs::read(p.join("latin1.txt")).unwrap(), b"caf\xE9\n");
    rt.state.close_file_edit();
    std::fs::write(p.join("b.txt"), "staged change\n").unwrap();
    git(p, &["add", "b.txt"]);
    rt.state.refresh();
    rt.state.set_view(crate::state::ViewKind::Staged);
    settle(&mut rt, |s| s.count(crate::state::ViewKind::Staged) > 0);
    for _ in 0..300 {
        step(&mut rt);
        if rect_of(&rt, "fh1").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        rect_of(&rt, "fh1").is_some(),
        "the staged file has a header"
    );
    assert!(rect_of(&rt, "ed1").is_none());
}

#[test]
fn a_late_save_answer_is_ignored() {
    let dir = repo();
    let mut rt = editor(dir.path());
    let stale = rt.state.file_edit.request;
    type_text(&mut rt, "typed\n");
    click(&mut rt, "fe-save");
    assert!(
        !rt.state
            .apply_file_edit_saved(stale, Err(("late".to_owned(), false)))
    );
    settle(&mut rt, |s| !s.file_edit.saving);
    assert!(rt.state.file_edit.save_error.is_none());
}

#[test]
fn closing_with_unsaved_changes_asks_first_and_discarding_leaves_the_disk_alone() {
    let dir = repo();
    let mut rt = editor(dir.path());
    type_text(&mut rt, "typed\n");
    click(&mut rt, "fe-close");
    step(&mut rt);
    assert!(rt.state.file_edit.open && rt.state.file_edit.confirm_discard);
    assert!(shows(&rt, "les abandonner"));
    click(&mut rt, "fe-continue");
    step(&mut rt);
    assert!(!rt.state.file_edit.confirm_discard && rt.state.file_edit.open);
    super::press(&mut rt, rune::Key::Escape);
    assert!(rt.state.file_edit.confirm_discard);
    click(&mut rt, "fe-discard");
    step(&mut rt);
    assert!(!rt.state.file_edit.open);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one\ntwo\nlocal\n"
    );
}

#[test]
fn a_read_only_file_keeps_the_text_and_reports_the_refusal() {
    let dir = repo();
    let mut rt = editor(dir.path());
    type_text(&mut rt, "wanted\n");
    std::fs::set_permissions(
        dir.path().join("a.txt"),
        std::fs::Permissions::from_mode(0o444),
    )
    .unwrap();
    click(&mut rt, "fe-save");
    settle(&mut rt, |s| {
        !s.file_edit.saving && s.file_edit.save_error.is_some()
    });
    assert!(shows(&rt, "accès refusé en écriture"));
    assert_eq!(rt.state.file_edit.text.text, "wanted\n");
    assert!(rt.state.file_edit.dirty());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one\ntwo\nlocal\n"
    );
}
