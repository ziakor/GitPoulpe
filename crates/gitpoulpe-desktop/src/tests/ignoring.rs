use std::path::{Path, PathBuf};
use std::process::Command;

use rune::Runtime;

use super::{click_at, rect_of, runtime, step};
use crate::ignored::ReasonState;
use crate::state::{State, ViewKind};

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
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn settle(rt: &mut Runtime<State>, until: impl Fn(&State) -> bool) {
    for _ in 0..80 {
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

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("tracked.log"), "one\n").unwrap();
    std::fs::write(p.join("code.txt"), "c\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "base"]);
    std::fs::write(p.join(".gitignore"), "*.log\n!keep.log\n").unwrap();
    std::fs::write(p.join("a.log"), "ignored\n").unwrap();
    std::fs::write(p.join("keep.log"), "kept\n").unwrap();
    std::fs::write(p.join("tracked.log"), "two\n").unwrap();
    std::fs::write(p.join("todo.dat"), "d\n").unwrap();
    dir
}

fn screen(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "ignored");
    settle(&mut rt, |s| s.ignored.open && !s.ignored.loading);
    rt
}

fn pick(rt: &mut Runtime<State>, path: &str) {
    rt.state.select_ignored(PathBuf::from(path));
    settle(rt, |s| {
        s.ignored
            .target
            .as_ref()
            .is_some_and(|t| t.reason != ReasonState::Loading)
    });
}

fn unstaged(rt: &Runtime<State>) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = rt
        .state
        .repo
        .as_ref()
        .unwrap()
        .snapshot
        .entries
        .iter()
        .filter(|e| e.is_unstaged())
        .map(|e| e.path.clone())
        .collect();
    paths.sort();
    paths
}

#[test]
fn the_screen_lists_ignored_files_and_explains_the_rule_that_excludes_each() {
    let dir = repo();
    let mut rt = screen(dir.path());
    assert_eq!(rt.state.ignored.paths, vec![PathBuf::from("a.log")]);
    pick(&mut rt, "a.log");
    assert!(shows(
        &rt,
        "Règle « *.log », ligne 1 de .gitignore à la racine"
    ));
    assert!(shows(&rt, "Le fichier est ignoré."));

    pick(&mut rt, "keep.log");
    assert!(shows(&rt, "Règle « !keep.log », ligne 2"));
    assert!(shows(&rt, "C'est une exception"));

    pick(&mut rt, "tracked.log");
    assert!(shows(&rt, "déjà suivi par Git"));
    assert!(shows(&rt, "git rm --cached"));

    pick(&mut rt, "todo.dat");
    assert!(shows(&rt, "Aucune règle ne concerne ce chemin."));
}

#[test]
fn ignoring_an_untracked_file_from_its_header_writes_only_the_chosen_file() {
    let dir = repo();
    let mut rt = runtime(super::opened(dir.path()));
    rt.on_key = Some(crate::keys::on_key);
    rt.state.set_view(ViewKind::Unstaged);
    for _ in 0..4 {
        step(&mut rt);
    }
    assert!(unstaged(&rt).contains(&PathBuf::from("todo.dat")));
    let gitignore = std::fs::read(dir.path().join(".gitignore")).unwrap();
    let exclude_before = std::fs::read_to_string(dir.path().join(".git/info/exclude")).unwrap();
    let position = rt
        .state
        .entries()
        .iter()
        .position(|e| e.path == Path::new("todo.dat"))
        .unwrap();
    rt.state.jump_to(position);
    for _ in 0..6 {
        step(&mut rt);
    }
    let (index, _) = rt.state.entry_at(position).unwrap();
    click(&mut rt, &format!("ig{index}"));
    settle(&mut rt, |s| {
        s.ignored.open
            && s.ignored
                .target
                .as_ref()
                .is_some_and(|t| t.reason != ReasonState::Loading)
    });
    assert!(shows(&rt, "Aucune règle ne concerne ce chemin."));
    assert!(shows(
        &rt,
        ".git/info/exclude (local, non partagé) : /todo.dat"
    ));

    click(&mut rt, "ignore-add-1");
    settle(&mut rt, |s| {
        s.ignored
            .target
            .as_ref()
            .is_some_and(|t| matches!(&t.reason, ReasonState::Known(Some(_))))
    });
    let exclude = std::fs::read_to_string(dir.path().join(".git/info/exclude")).unwrap();
    assert!(
        exclude.starts_with(&exclude_before),
        "existing lines are kept"
    );
    assert_eq!(&exclude[exclude_before.len()..], "/todo.dat\n");
    assert_eq!(
        std::fs::read(dir.path().join(".gitignore")).unwrap(),
        gitignore,
        "the shared .gitignore was not touched"
    );
    assert!(!unstaged(&rt).contains(&PathBuf::from("todo.dat")));
    assert!(rt.state.ignored.paths.contains(&PathBuf::from("todo.dat")));
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.starts_with("Règle /todo.dat ajoutée à ") && n.ends_with("exclude.")),
        "{:?}",
        rt.state.notice
    );
}

#[test]
fn removing_the_rule_from_its_own_file_makes_the_file_visible_again() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "a.log");
    click(&mut rt, "ignore-remove");
    settle(&mut rt, |s| {
        s.ignored
            .target
            .as_ref()
            .is_some_and(|t| t.reason == ReasonState::Known(None))
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".gitignore")).unwrap(),
        "!keep.log\n"
    );
    settle(&mut rt, |s| {
        s.repo
            .as_ref()
            .unwrap()
            .snapshot
            .entries
            .iter()
            .any(|e| e.path == Path::new("a.log"))
    });
    assert!(unstaged(&rt).contains(&PathBuf::from("a.log")));
}

#[test]
fn a_rule_added_for_a_tracked_file_leaves_it_tracked_and_still_modified() {
    let dir = repo();
    let before_index = git(dir.path(), &["ls-files", "-s"]);
    let mut rt = screen(dir.path());
    pick(&mut rt, "tracked.log");
    click(&mut rt, "ignore-add-0");
    settle(&mut rt, |s| {
        std::fs::read_to_string(
            s.repo
                .as_ref()
                .map(|_| dir.path().join(".gitignore"))
                .unwrap(),
        )
        .unwrap()
        .contains("/tracked.log")
    });
    for _ in 0..30 {
        step(&mut rt);
    }
    assert_eq!(git(dir.path(), &["ls-files", "-s"]), before_index);
    assert!(
        unstaged(&rt).contains(&PathBuf::from("tracked.log")),
        "the modified tracked file is still listed as modified"
    );
    assert!(
        !rt.state
            .ignored
            .paths
            .contains(&PathBuf::from("tracked.log"))
    );
}

#[test]
fn escape_closes_the_screen_and_the_other_screens_close_it() {
    let dir = repo();
    let mut rt = screen(dir.path());
    super::press(&mut rt, rune::Key::Escape);
    assert!(!rt.state.ignored.open);
    click(&mut rt, "ignored");
    settle(&mut rt, |s| s.ignored.open);
    click(&mut rt, "history");
    settle(&mut rt, |s| s.history_open);
    assert!(!rt.state.ignored.open);
}
