use std::path::Path;
use std::process::Command;

use rune::{Key, KeyEvent, Mods, Runtime};

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
    assert!(output.status.success(), "{args:?}");
}

fn repo(file: &str, dirty: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join(file), "one\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    if dirty {
        std::fs::write(p.join(file), "one\ntwo\n").unwrap();
    }
    dir
}

fn app(state: State) -> Runtime<State> {
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    rt
}

fn ctrl(key: char) -> KeyEvent {
    KeyEvent {
        key: Key::Command(key),
        mods: Mods {
            ctrl: true,
            cmd: true,
            ..Mods::default()
        },
    }
}

fn open_palette(rt: &mut Runtime<State>) {
    rt.key(ctrl('k'));
    step(rt);
}

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    step(rt);
}

fn type_text(rt: &mut Runtime<State>, text: &str) {
    for ch in text.chars() {
        press(rt, Key::Text(ch.to_string()));
    }
}

fn labels(rt: &Runtime<State>) -> Vec<&'static str> {
    rt.state.palette_results().iter().map(|c| c.label).collect()
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused)
        .and_then(|e| e.key)
}

fn wait_for(rt: &mut Runtime<State>, what: &str, until: impl Fn(&State) -> bool) {
    for _ in 0..400 {
        step(rt);
        if until(&rt.state) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("never saw {what}");
}

#[test]
fn without_a_repository_only_the_commands_that_work_there_are_offered() {
    let mut rt = app(State::new(None, rune::prefs::Prefs::memory()));
    open_palette(&mut rt);
    assert!(rt.state.commands.open);
    let all = labels(&rt);
    assert!(all.contains(&"Ouvrir un dépôt…") && all.contains(&"Cloner un dépôt"));
    assert!(!all.contains(&"Branches") && !all.contains(&"Historique des commits"));
    type_text(&mut rt, "branch");
    assert!(labels(&rt).is_empty());
    assert!(rect_of(&rt, "pal-empty").is_some());
}

#[test]
fn a_clean_repository_offers_repository_commands_but_none_on_a_file() {
    let dir = repo("a.txt", false);
    let mut rt = app(super::opened(dir.path()));
    open_palette(&mut rt);
    let all = labels(&rt);
    assert!(all.contains(&"Branches") && all.contains(&"Actualiser le dépôt"));
    assert!(!all.contains(&"Historique du fichier courant"));
    assert!(!all.contains(&"Éditer le fichier courant"));
    assert!(!all.contains(&"Résoudre les conflits"));
}

#[test]
fn a_modified_repository_offers_file_commands_that_follow_the_view() {
    let dir = repo("a.txt", true);
    let mut rt = app(super::opened(dir.path()));
    wait_for(&mut rt, "the current file", |s| s.current.is_some());
    open_palette(&mut rt);
    let all = labels(&rt);
    assert!(all.contains(&"Historique du fichier courant"));
    assert!(
        all.contains(&"Éditer le fichier courant") && all.contains(&"Indexer le fichier courant")
    );
    assert!(!all.contains(&"Désindexer le fichier courant"));
    rt.key(ctrl('k'));
    step(&mut rt);
    assert!(!rt.state.commands.open);
    git(dir.path(), &["add", "a.txt"]);
    rt.state.refresh();
    wait_for(&mut rt, "the staged count", |s| {
        s.count(crate::state::ViewKind::Staged) > 0
    });
    rt.state.set_view(crate::state::ViewKind::Staged);
    wait_for(&mut rt, "a current staged file", |s| {
        s.view == crate::state::ViewKind::Staged
            && s.current
                .and_then(|p| s.entry_at(p))
                .is_some_and(|(_, e)| e.is_staged())
    });
    open_palette(&mut rt);
    wait_for(&mut rt, "the unstage command", |s| {
        s.palette_results()
            .iter()
            .any(|c| c.label == "Désindexer le fichier courant")
    });
    let all = labels(&rt);
    assert!(
        !all.contains(&"Éditer le fichier courant") && !all.contains(&"Indexer le fichier courant")
    );
}

#[test]
fn the_keyboard_alone_searches_navigates_runs_and_cancels_with_the_focus_back() {
    let dir = repo("a.txt", true);
    let mut rt = app(super::opened(dir.path()));
    press(&mut rt, Key::Text("/".to_owned()));
    assert_eq!(focused(&rt).as_deref(), Some("filter"));
    open_palette(&mut rt);
    assert_eq!(focused(&rt).as_deref(), Some("pal-input"));
    type_text(&mut rt, "etiquettes");
    assert_eq!(labels(&rt), ["Étiquettes (tags)"]);
    press(&mut rt, Key::Escape);
    assert!(!rt.state.commands.open);
    assert_eq!(focused(&rt).as_deref(), Some("filter"));
    open_palette(&mut rt);
    type_text(&mut rt, "fichier");
    assert!(labels(&rt).len() >= 3);
    assert_eq!(labels(&rt)[0], "Fichiers ignorés");
    let first = rt.state.palette_selected_id().unwrap();
    press(&mut rt, Key::Down);
    assert_ne!(rt.state.palette_selected_id().unwrap(), first);
    press(&mut rt, Key::Up);
    assert_eq!(rt.state.palette_selected_id().unwrap(), first);
    press(&mut rt, Key::Up);
    assert_eq!(
        rt.state.palette_selected_id().unwrap(),
        *labels_ids(&rt).last().unwrap()
    );
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Enter);
    assert!(!rt.state.commands.open);
    wait_for(&mut rt, "the ignored files screen", |s| s.ignored.open);
}

fn labels_ids(rt: &Runtime<State>) -> Vec<&'static str> {
    rt.state.palette_results().iter().map(|c| c.id).collect()
}

#[test]
fn accents_case_and_keywords_all_find_the_command() {
    let dir = repo("a.txt", false);
    let mut rt = app(super::opened(dir.path()));
    open_palette(&mut rt);
    type_text(&mut rt, "ÉTIQUETTE");
    assert_eq!(labels(&rt), ["Étiquettes (tags)"]);
    for _ in 0.."ÉTIQUETTE".chars().count() {
        press(&mut rt, Key::Backspace);
    }
    type_text(&mut rt, "release");
    assert!(labels(&rt).contains(&"Étiquettes (tags)"));
    for _ in 0.."release".chars().count() {
        press(&mut rt, Key::Backspace);
    }
    type_text(&mut rt, "stash remis");
    assert_eq!(labels(&rt), ["Remisages (stash)"]);
}

#[test]
fn a_command_runs_against_the_repository_and_file_current_when_it_is_launched() {
    let first = repo("first_only.txt", true);
    let second = repo("second_only.txt", true);
    let mut rt = app(super::opened(first.path()));
    wait_for(&mut rt, "the first file", |s| s.current.is_some());
    open_palette(&mut rt);
    type_text(&mut rt, "historique du fichier");
    assert_eq!(labels(&rt), ["Historique du fichier courant"]);
    rt.state.open_path(second.path().to_path_buf());
    wait_for(&mut rt, "the second repository", |s| {
        s.repo.as_ref().is_some_and(|r| {
            r.snapshot
                .entries
                .iter()
                .any(|e| e.path.ends_with("second_only.txt"))
        })
    });
    assert!(rt.state.commands.open);
    wait_for(&mut rt, "the command for the new repository's file", |s| {
        !s.palette_results().is_empty()
    });
    press(&mut rt, Key::Enter);
    wait_for(&mut rt, "the file history", |s| s.file_history.open);
    assert_eq!(rt.state.file_history.path, "second_only.txt");
    assert!(!rt.state.commands.open);
}

#[test]
fn a_command_that_stops_being_available_is_not_run_blindly() {
    let dir = repo("a.txt", true);
    let mut rt = app(super::opened(dir.path()));
    wait_for(&mut rt, "the current file", |s| s.current.is_some());
    open_palette(&mut rt);
    type_text(&mut rt, "editer");
    assert_eq!(labels(&rt), ["Éditer le fichier courant"]);
    git(dir.path(), &["checkout", "--", "a.txt"]);
    rt.state.refresh();
    wait_for(&mut rt, "the clean status", |s| {
        s.count(crate::state::ViewKind::Unstaged) == 0
    });
    assert!(labels(&rt).is_empty());
    press(&mut rt, Key::Enter);
    assert!(!rt.state.file_edit.open);
}

#[test]
fn clicking_a_result_runs_it() {
    let dir = repo("a.txt", false);
    let mut rt = app(super::opened(dir.path()));
    open_palette(&mut rt);
    type_text(&mut rt, "reflog");
    let rect = rect_of(&rt, "pal-row0").expect("the result is on screen");
    click_at(&mut rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
    wait_for(&mut rt, "the reflog screen", |s| s.reflog.open);
    assert!(!rt.state.commands.open);
}
