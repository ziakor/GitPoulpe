use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use rune::Runtime;

use super::{click_at, rect_of, runtime, step};
use crate::history_state::HistoryStatus;
use crate::state::{State, ViewKind};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.org")
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

struct Fixture {
    repo: tempfile::TempDir,
    initial: String,
    ordinary: String,
    special: String,
    merge: String,
}

fn fixture() -> Fixture {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path();
    git(dir, &["init", "-q", "-b", "main"]);
    let commit = |message: &str| {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", message]);
        git(dir, &["rev-parse", "HEAD"])
    };
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(
        dir.join("old name.txt"),
        (1..=30).map(|n| format!("line {n}\n")).collect::<String>(),
    )
    .unwrap();
    let initial = commit("initial commit");
    std::fs::write(dir.join("a.txt"), "one\ntwo changed\nthree\nfour\n").unwrap();
    std::fs::write(dir.join("c.txt"), "brand new\n").unwrap();
    std::fs::rename(dir.join("old name.txt"), dir.join("new name.txt")).unwrap();
    let ordinary = commit("ordinary commit\n\nthe body explains why");
    std::fs::write(dir.join("blob.bin"), b"\0\x01binary").unwrap();
    std::fs::write(
        dir.join(OsString::from_vec(b"bytes-\xff.txt".to_vec())),
        "raw bytes name\n",
    )
    .unwrap();
    std::fs::write(dir.join("with space.txt"), "spaced name content\n").unwrap();
    let special = commit("special paths");
    git(dir, &["checkout", "-q", "-b", "side", &ordinary]);
    std::fs::write(dir.join("side.txt"), "from the side\n").unwrap();
    commit("side work");
    git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("main-only.txt"), "main work\n").unwrap();
    commit("main work");
    git(dir, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    let merge = git(dir, &["rev-parse", "HEAD"]);
    Fixture {
        repo,
        initial,
        ordinary,
        special,
        merge,
    }
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn inspect(rt: &mut Runtime<State>, oid: &str) {
    let index = rt
        .state
        .history
        .commits
        .iter()
        .position(|c| c.oid == oid)
        .expect("the commit is in the loaded history");
    rt.state.select_commit(index);
    for _ in 0..3 {
        step(rt);
    }
    let button = rect_of(rt, "inspect-commit").expect("the detail panel offers the inspection");
    click_at(rt, button[0] + 10.0, button[1] + button[3] / 2.0, false);
    for _ in 0..60 {
        step(rt);
        if rt.state.view == ViewKind::Commit
            && rt
                .state
                .inspected
                .as_ref()
                .is_some_and(|i| i.detail.commit.oid == oid)
        {
            break;
        }
    }
    for _ in 0..12 {
        step(rt);
    }
}

fn history(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    let button = rect_of(&rt, "history").unwrap();
    click_at(
        &mut rt,
        button[0] + 10.0,
        button[1] + button[3] / 2.0,
        false,
    );
    for _ in 0..60 {
        step(&mut rt);
        if rt.state.history.status == HistoryStatus::Ready {
            break;
        }
    }
    for _ in 0..4 {
        step(&mut rt);
    }
    rt
}

fn listed(rt: &Runtime<State>) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = rt.state.entries().iter().map(|e| e.path.clone()).collect();
    paths.sort();
    paths
}

fn git_names(dir: &Path, base: &str, oid: &str) -> Vec<PathBuf> {
    let output = Command::new("git")
        .current_dir(dir)
        .args(["diff-tree", "-r", "-z", "--name-only", "-M", base, oid])
        .output()
        .unwrap();
    let mut names: Vec<PathBuf> = output
        .stdout
        .split(|b| *b == 0)
        .filter(|n| !n.is_empty())
        .map(|n| PathBuf::from(OsString::from_vec(n.to_vec())))
        .collect();
    names.sort();
    names
}

#[test]
fn an_ordinary_commit_lists_its_files_and_reads_their_diffs_in_the_continuous_view() {
    let f = fixture();
    let mut rt = history(f.repo.path());
    inspect(&mut rt, &f.ordinary);

    assert_eq!(rt.state.view, ViewKind::Commit);
    assert_eq!(
        listed(&rt),
        git_names(f.repo.path(), &f.initial, &f.ordinary)
    );
    assert!(shows(&rt, "ordinary commit"), "the card names the commit");
    assert!(shows(&rt, "Ada Lovelace"), "and its author");
    assert!(shows(&rt, "the body explains why"), "and its message");
    assert!(
        shows(&rt, "two changed"),
        "the diff of the first file is in the continuous stream"
    );
    let renamed = rt
        .state
        .entries()
        .iter()
        .find(|e| e.path == Path::new("new name.txt"))
        .unwrap();
    assert_eq!(
        renamed.original_path.as_deref(),
        Some(Path::new("old name.txt"))
    );
}

#[test]
fn the_first_commit_is_inspectable_against_an_empty_tree() {
    let f = fixture();
    let mut rt = history(f.repo.path());
    inspect(&mut rt, &f.initial);
    assert_eq!(rt.state.view, ViewKind::Commit);
    assert_eq!(
        listed(&rt),
        vec![PathBuf::from("a.txt"), PathBuf::from("old name.txt")]
    );
    assert!(shows(&rt, "Premier commit : comparé à un arbre vide"));
    assert!(shows(&rt, "one"), "its content is shown as additions");
}

#[test]
fn a_merge_names_its_parents_and_changes_the_file_list_with_the_chosen_one() {
    let f = fixture();
    let dir = f.repo.path();
    let mut rt = history(dir);
    inspect(&mut rt, &f.merge);
    let parents = rt
        .state
        .inspected
        .as_ref()
        .unwrap()
        .detail
        .commit
        .parents
        .clone();
    assert_eq!(parents.len(), 2);
    assert!(shows(&rt, "choisis le parent de comparaison"));
    assert_eq!(rt.state.inspected.as_ref().unwrap().base_index, 0);
    let against_first = listed(&rt);
    assert_eq!(against_first, git_names(dir, &parents[0], &f.merge));

    let second = rect_of(&rt, "parent-1").expect("the second parent can be chosen");
    click_at(
        &mut rt,
        second[0] + 10.0,
        second[1] + second[3] / 2.0,
        false,
    );
    for _ in 0..60 {
        step(&mut rt);
        if rt
            .state
            .inspected
            .as_ref()
            .is_some_and(|i| i.base_index == 1)
        {
            break;
        }
    }
    for _ in 0..8 {
        step(&mut rt);
    }
    let against_second = listed(&rt);
    assert_eq!(against_second, git_names(dir, &parents[1], &f.merge));
    assert_ne!(
        against_first, against_second,
        "the comparison really changed"
    );
    assert!(against_first.contains(&PathBuf::from("side.txt")));
    assert!(against_second.contains(&PathBuf::from("main-only.txt")));
}

#[test]
fn binary_files_and_unusual_paths_keep_their_meaning_in_the_stream() {
    let f = fixture();
    let mut rt = history(f.repo.path());
    inspect(&mut rt, &f.special);
    let names = listed(&rt);
    assert!(names.contains(&PathBuf::from("blob.bin")));
    assert!(names.contains(&PathBuf::from("with space.txt")));
    assert!(names.contains(&PathBuf::from(OsString::from_vec(
        b"bytes-\xff.txt".to_vec()
    ))));

    for (path, needle) in [
        ("blob.bin", "Fichier binaire"),
        ("with space.txt", "spaced name content"),
    ] {
        let position = rt
            .state
            .files
            .iter()
            .position(|&i| rt.state.entries()[i].path == Path::new(path))
            .unwrap();
        rt.update(move |s, tx| {
            s.jump_to(position);
            crate::keys::align_file(tx, s.files[position]);
        });
        for _ in 0..10 {
            step(&mut rt);
        }
        assert!(shows(&rt, needle), "{path} shows {needle:?}");
    }
}

#[test]
fn inspecting_offers_no_write_action_and_leaving_returns_to_the_history() {
    let f = fixture();
    let mut rt = history(f.repo.path());
    inspect(&mut rt, &f.ordinary);
    let write_keys = rt
        .elements()
        .into_iter()
        .filter_map(|e| e.key)
        .filter(|k| {
            ["st", "un", "hb", "gs", "mk", "sl", "stage-marked"]
                .iter()
                .any(|p| k.starts_with(p))
                && !k.starts_with("start")
        })
        .collect::<Vec<_>>();
    assert!(
        write_keys.is_empty(),
        "no staging control in the commit view: {write_keys:?}"
    );
    assert!(rect_of(&rt, "commit").is_none(), "no commit box either");
    assert!(rect_of(&rt, "commit-message").is_none());

    super::press(&mut rt, rune::Key::Text("s".to_owned()));
    assert!(!rt.state.writing, "the stage shortcut does nothing here");

    super::press(&mut rt, rune::Key::Escape);
    for _ in 0..30 {
        step(&mut rt);
        if rt.state.history_open {
            break;
        }
    }
    assert_eq!(rt.state.view, ViewKind::Unstaged);
    assert!(rt.state.inspected.is_none());
    assert!(
        rt.state.history_open,
        "back to the history the user came from"
    );
}
