use std::path::{Path, PathBuf};
use std::process::Command;

use gitpoulpe_core::Favorite;
use rune::{Key, KeyEvent, Mods, Runtime};

use super::{click_at, focused, rect_of, runtime, step};
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

fn init(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "init"]);
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path());
    dir
}

fn named(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir(&dir).unwrap();
    init(&dir);
    dir
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap()
}

fn favorite(path: &Path, name: &str) -> Favorite {
    Favorite::Local {
        path: path.display().to_string(),
        name: name.to_owned(),
    }
}

fn app(current: &Path, favorites: Vec<Favorite>) -> Runtime<State> {
    let mut state = super::opened(current);
    state.favorites.book.items = favorites;
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    rt
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
    for _ in 0..3 {
        step(rt);
    }
}

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    for _ in 0..3 {
        step(rt);
    }
}

fn opened_path(rt: &Runtime<State>) -> Option<PathBuf> {
    rt.state.repo.as_ref().map(|r| canonical(&r.path))
}

fn settle_on(rt: &mut Runtime<State>, path: &Path) {
    for _ in 0..400 {
        step(rt);
        if opened_path(rt).as_deref() == Some(path) && !rt.state.files.is_empty() {
            return;
        }
    }
    panic!("{} never opened", path.display());
}

#[test]
fn the_repository_name_lists_the_other_known_repositories_and_opens_one_in_a_click() {
    let here = repo();
    let other = repo();
    let mut rt = app(
        here.path(),
        vec![
            favorite(here.path(), "ici"),
            favorite(other.path(), "autre"),
        ],
    );
    click(&mut rt, "repo-switch");
    assert!(rt.state.switcher.open);
    assert_eq!(focused(&rt).as_deref(), Some("sw-input"));
    let listed: Vec<String> = rt
        .state
        .switcher_results()
        .iter()
        .map(|c| c.path.clone())
        .collect();
    assert_eq!(
        listed,
        vec![canonical(other.path()).display().to_string()],
        "the open repository is not offered again"
    );
    std::fs::write(other.path().join("b.txt"), "new\n").unwrap();
    click(&mut rt, "sw-row0");
    settle_on(&mut rt, &canonical(other.path()));
    assert!(!rt.state.switcher.open);
}

#[test]
fn the_switcher_is_driven_from_the_keyboard_and_gives_the_focus_back() {
    let here = repo();
    let root = tempfile::tempdir().unwrap();
    let second = named(root.path(), "beta");
    let first = named(root.path(), "alpha");
    let mut rt = app(
        here.path(),
        vec![favorite(&second, "beta"), favorite(&first, "alpha")],
    );
    super::focus(&mut rt, "filter");
    rt.update(|s, tx| s.open_switcher(tx));
    step(&mut rt);
    let order: Vec<String> = rt
        .state
        .switcher_results()
        .iter()
        .map(|c| c.path.clone())
        .collect();
    assert_eq!(
        order,
        vec![
            canonical(&first).display().to_string(),
            canonical(&second).display().to_string()
        ],
        "sorted by name"
    );
    press(&mut rt, Key::Down, Mods::default());
    assert_eq!(rt.state.switcher_selected(), Some(order[1].clone()));
    press(&mut rt, Key::Down, Mods::default());
    assert_eq!(
        rt.state.switcher_selected(),
        Some(order[0].clone()),
        "wraps"
    );
    press(&mut rt, Key::Escape, Mods::default());
    assert!(!rt.state.switcher.open);
    assert_eq!(focused(&rt).as_deref(), Some("filter"));
    rt.update(|s, tx| s.open_switcher(tx));
    step(&mut rt);
    press(&mut rt, Key::Up, Mods::default());
    let target = PathBuf::from(&order[1]);
    std::fs::write(target.join("c.txt"), "new\n").unwrap();
    press(&mut rt, Key::Enter, Mods::default());
    settle_on(&mut rt, &target);
}

#[test]
fn a_vanished_repository_is_explained_and_nothing_else_is_opened() {
    let here = repo();
    let gone = tempfile::tempdir().unwrap();
    let missing = gone.path().join("ancien-projet");
    let mut rt = app(here.path(), vec![favorite(&missing, "ancien-projet")]);
    click(&mut rt, "repo-switch");
    click(&mut rt, "sw-row0");
    assert!(rt.state.switcher.open);
    assert_eq!(opened_path(&rt), Some(canonical(here.path())));
    let notice = rt.state.switcher.notice.clone().unwrap();
    assert!(notice.contains("introuvable"), "{notice}");
    assert!(rect_of(&rt, "sw-notice").is_some());
}

#[test]
fn the_open_repository_can_be_added_to_the_favorites_from_the_switcher() {
    let here = repo();
    let mut rt = app(here.path(), Vec::new());
    click(&mut rt, "repo-switch");
    assert!(rect_of(&rt, "sw-empty").is_some());
    click(&mut rt, "sw-favorite");
    assert!(rt.state.current_is_favorite());
    assert!(rect_of(&rt, "sw-favorite").is_none());
    let notice = rt.state.switcher.notice.clone().unwrap();
    assert!(notice.ends_with("ajouté aux favoris."), "{notice}");
}

#[test]
fn the_palette_and_the_switcher_never_show_together() {
    let here = repo();
    let mut rt = app(here.path(), Vec::new());
    click(&mut rt, "repo-switch");
    press(
        &mut rt,
        Key::Command('k'),
        Mods {
            ctrl: true,
            cmd: true,
            ..Mods::default()
        },
    );
    assert!(rt.state.commands.open);
    assert!(!rt.state.switcher.open);
    assert!(rect_of(&rt, "sw-bar").is_none());
    click(&mut rt, "repo-switch");
    assert!(rt.state.switcher.open);
    assert!(!rt.state.commands.open);
}
