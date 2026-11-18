use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use rune::{Key, Runtime, TextEdit};

use super::{click_at, press, rect_of, runtime, step};
use crate::state::{Screen, State};

fn head_branch(dir: &Path) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn settle(rt: &mut Runtime<State>, until: impl Fn(&State) -> bool) {
    for _ in 0..120 {
        step(rt);
        if until(&rt.state) {
            break;
        }
    }
    for _ in 0..4 {
        step(rt);
    }
    assert!(until(&rt.state), "the screen never settled");
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn welcome() -> Runtime<State> {
    let mut state = State::new(None, rune::prefs::Prefs::memory());
    state.setup(Arc::new(|| {}));
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    rt
}

fn ready(s: &State) -> bool {
    s.screen == Screen::Ready && s.repo.is_some()
}

#[test]
fn the_welcome_screen_offers_to_initialise_and_a_new_repository_opens_without_a_commit() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), "hello\n").unwrap();
    let mut rt = welcome();
    assert!(rect_of(&rt, "init").is_some());
    rt.state.begin_init(dir.path().to_path_buf());
    settle(&mut rt, |s| s.init.open && !s.init.branch.text.is_empty());
    assert!(shows(&rt, "Initialiser un dépôt"));
    rt.state.init.branch = TextEdit::new("trunk");
    step(&mut rt);
    click(&mut rt, "init-confirm");
    settle(&mut rt, ready);
    assert!(!rt.state.init.open);
    assert_eq!(head_branch(dir.path()), "trunk");
    let repo = rt.state.repo.as_ref().unwrap();
    assert_eq!(repo.snapshot.entries.len(), 1);
    assert_eq!(repo.snapshot.branch.oid.as_deref(), Some("(initial)"));
}

#[test]
fn an_existing_repository_is_explained_not_reinitialised_and_the_panel_stays_usable() {
    let dir = tempfile::tempdir().unwrap();
    Command::new("git")
        .current_dir(dir.path())
        .args(["init", "-q", "-b", "main"])
        .output()
        .unwrap();
    let mut rt = welcome();
    rt.state.begin_init(dir.path().to_path_buf());
    settle(&mut rt, |s| s.init.open);
    click(&mut rt, "init-confirm");
    settle(&mut rt, |s| s.init.error.is_some() && !s.init.busy);
    assert!(shows(&rt, "déjà un dépôt Git"));
    assert!(rt.state.repo.is_none());
    assert_eq!(head_branch(dir.path()), "main");
    press(&mut rt, Key::Escape);
    press(&mut rt, Key::Escape);
    assert!(!rt.state.init.open);
}

#[test]
fn a_refused_attempt_can_be_retried_after_fixing_the_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = welcome();
    rt.state.begin_init(dir.path().to_path_buf());
    settle(&mut rt, |s| s.init.open);
    rt.state.init.branch = TextEdit::new("bad name");
    step(&mut rt);
    click(&mut rt, "init-confirm");
    settle(&mut rt, |s| s.init.error.is_some() && !s.init.busy);
    assert!(shows(&rt, "nom de branche initiale invalide"));
    assert!(!dir.path().join(".git").exists());
    rt.state.init.branch = TextEdit::new("main");
    step(&mut rt);
    assert!(shows(&rt, "Réessayer"));
    click(&mut rt, "init-confirm");
    settle(&mut rt, ready);
    assert!(dir.path().join(".git").exists());
}

#[test]
fn a_folder_that_failed_to_open_as_a_repository_offers_to_initialise_that_folder() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = welcome();
    rt.state.open_path(dir.path().to_path_buf());
    settle(&mut rt, |s| matches!(s.screen, Screen::Failed { .. }));
    click(&mut rt, "init-here");
    settle(&mut rt, |s| s.init.open);
    assert_eq!(rt.state.init.path, dir.path());
}
