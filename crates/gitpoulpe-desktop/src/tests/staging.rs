use std::path::{Path, PathBuf};
use std::process::Command;

use super::{click_at, keyboard_runtime, press, pump, rect_of, step};
use crate::state::{State, ViewKind};

fn git_names(dir: &Path, args: &[&str]) -> Vec<String> {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .unwrap();
    let mut names: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    names.sort();
    names
}

fn staged(dir: &Path) -> Vec<String> {
    git_names(dir, &["diff", "--cached", "--name-only"])
}

fn path_at(state: &State, position: usize) -> PathBuf {
    state.entry_at(position).unwrap().1.path.clone()
}

fn finish_writing(rt: &mut rune::Runtime<State>) {
    for _ in 0..200 {
        step(rt);
        if !rt.state.writing {
            break;
        }
    }
    for _ in 0..6 {
        step(rt);
    }
    assert!(!rt.state.writing, "the write never finished");
}

#[test]
fn the_s_key_stages_the_current_file_and_the_views_follow() {
    let (repo, mut rt) = keyboard_runtime(5);
    let before = staged(repo.path());
    let unstaged = rt.state.count(ViewKind::Unstaged);
    let path = path_at(&rt.state, 1);
    rt.state.jump_to(1);

    press(&mut rt, rune::Key::Text("s".to_owned()));
    finish_writing(&mut rt);

    let after = staged(repo.path());
    assert_eq!(after.len(), before.len() + 1, "{after:?}");
    assert!(after.contains(&path.to_string_lossy().into_owned()));
    assert_eq!(rt.state.count(ViewKind::Unstaged), unstaged - 1);
    assert_eq!(rt.state.notice.as_deref(), Some("1 fichier indexé."));
}

#[test]
fn marked_files_are_staged_together_and_the_marks_are_cleared() {
    let (repo, mut rt) = keyboard_runtime(6);
    let before = staged(repo.path());
    let chosen = [path_at(&rt.state, 0), path_at(&rt.state, 2)];

    for position in [0, 2] {
        rt.state.jump_to(position);
        press(&mut rt, rune::Key::Text("x".to_owned()));
    }
    assert_eq!(rt.state.marked.len(), 2);
    assert!(
        rect_of(&rt, "stage-marked").is_some(),
        "the selection bar appears"
    );

    press(&mut rt, rune::Key::Text("S".to_owned()));
    finish_writing(&mut rt);

    let after = staged(repo.path());
    assert_eq!(after.len(), before.len() + 2);
    for path in &chosen {
        assert!(after.contains(&path.to_string_lossy().into_owned()));
    }
    assert!(rt.state.marked.is_empty());
    assert_eq!(rt.state.notice.as_deref(), Some("2 fichiers indexés."));
}

#[test]
fn the_button_in_a_file_header_stages_that_file() {
    let (repo, mut rt) = keyboard_runtime(4);
    let before = staged(repo.path());
    let (index, entry) = rt.state.entry_at(0).unwrap();
    let path = entry.path.to_string_lossy().into_owned();
    for _ in 0..4 {
        step(&mut rt);
    }
    let rect = rect_of(&rt, &format!("st{index}")).expect("the header button is mounted");
    click_at(
        &mut rt,
        rect[0] + rect[2] / 2.0,
        rect[1] + rect[3] / 2.0,
        false,
    );
    finish_writing(&mut rt);

    let after = staged(repo.path());
    assert_eq!(after.len(), before.len() + 1);
    assert!(after.contains(&path));
}

#[test]
fn a_file_edited_after_it_was_displayed_is_not_staged_and_the_user_is_told() {
    let (repo, mut rt) = keyboard_runtime(3);
    let before = staged(repo.path());
    let path = path_at(&rt.state, 0);
    std::fs::write(
        repo.path().join(&path),
        "rewritten after the diff was shown\n",
    )
    .unwrap();
    rt.state.jump_to(0);

    press(&mut rt, rune::Key::Text("s".to_owned()));
    finish_writing(&mut rt);

    assert_eq!(staged(repo.path()), before, "nothing was staged");
    let notice = rt.state.notice.clone().unwrap_or_default();
    assert!(notice.contains("a changé depuis son affichage"), "{notice}");
}

#[test]
fn a_second_request_while_one_is_running_is_refused_not_queued_silently() {
    let (_repo, mut rt) = keyboard_runtime(4);
    let first = path_at(&rt.state, 0);
    let second = path_at(&rt.state, 1);
    rt.state.stage_paths(vec![first]);
    assert!(rt.state.writing);
    rt.state.stage_paths(vec![second]);
    assert_eq!(
        rt.state.notice.as_deref(),
        Some("Une écriture est déjà en cours.")
    );
    finish_writing(&mut rt);
}

#[test]
fn staging_is_only_offered_for_unstaged_files() {
    let (_repo, mut rt) = keyboard_runtime(3);
    rt.state.set_view(ViewKind::Staged);
    for _ in 0..4 {
        step(&mut rt);
    }
    let any_button = rt.elements().into_iter().any(|e| {
        e.key
            .as_deref()
            .is_some_and(|k| k.starts_with("st") && k != "stage-marked")
    });
    assert!(!any_button, "no staging button in the staged view");
    let before = rt.state.notice.clone();
    rt.state.jump_to(0);
    press(&mut rt, rune::Key::Text("s".to_owned()));
    assert!(
        !rt.state.writing,
        "the s key does nothing in the staged view"
    );
    assert_eq!(rt.state.notice, before);
    pump(&mut rt.state, |_| true);
}

fn three_block_repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path();
    super::git(dir, &["init", "-q", "-b", "main"]);
    let base: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    std::fs::write(dir.join("f.txt"), &base).unwrap();
    super::git(dir, &["add", "-A"]);
    super::git(dir, &["commit", "-q", "-m", "base"]);
    let edited = base
        .replace("line 3\n", "line 3 CHANGED\n")
        .replace("line 20\n", "line 20 CHANGED\n")
        .replace("line 38\n", "line 38 CHANGED\n");
    std::fs::write(dir.join("f.txt"), edited).unwrap();
    repo
}

fn blocks_runtime(repo: &tempfile::TempDir) -> rune::Runtime<State> {
    let mut rt = super::runtime(super::opened(repo.path()));
    rt.on_key = Some(crate::keys::on_key);
    rt.state.settle(1, std::time::Duration::from_secs(20));
    for _ in 0..8 {
        step(&mut rt);
    }
    rt
}

fn click_block_button(rt: &mut rune::Runtime<State>, hunk: usize) {
    let index = rt.state.entry_at(0).unwrap().0;
    let key = format!("hb{index}:{hunk}");
    let rect = rect_of(rt, &key).unwrap_or_else(|| panic!("{key} is not mounted"));
    click_at(rt, rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0, false);
}

fn index_text(dir: &Path) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(["show", ":f.txt"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn the_block_button_stages_only_that_block_and_the_rest_stays_unstaged() {
    let repo = three_block_repo();
    let mut rt = blocks_runtime(&repo);

    click_block_button(&mut rt, 1);
    finish_writing(&mut rt);

    let staged_text = index_text(repo.path());
    assert!(staged_text.contains("line 20 CHANGED"));
    assert!(!staged_text.contains("line 3 CHANGED") && !staged_text.contains("line 38 CHANGED"));
    assert_eq!(rt.state.notice.as_deref(), Some("Bloc indexé."));
    assert_eq!(rt.state.count(ViewKind::Staged), 1);
    assert_eq!(
        rt.state.count(ViewKind::Unstaged),
        1,
        "still partly unstaged"
    );
}

#[test]
fn the_block_button_also_works_in_split_mode() {
    let repo = three_block_repo();
    let mut rt = blocks_runtime(&repo);
    rt.update(|s, tx| s.toggle_mode(tx));
    for _ in 0..8 {
        step(&mut rt);
    }
    assert_eq!(rt.state.mode, crate::state::DiffMode::Split);

    click_block_button(&mut rt, 0);
    finish_writing(&mut rt);

    let staged_text = index_text(repo.path());
    assert!(staged_text.contains("line 3 CHANGED"));
    assert!(!staged_text.contains("line 20 CHANGED"));
}

#[test]
fn a_block_of_a_file_edited_after_the_diff_was_shown_is_refused() {
    let repo = three_block_repo();
    let mut rt = blocks_runtime(&repo);
    let before = index_text(repo.path());
    let edited = std::fs::read_to_string(repo.path().join("f.txt"))
        .unwrap()
        .replace("line 20 CHANGED", "line 20 CHANGED ELSEWHERE");
    std::fs::write(repo.path().join("f.txt"), edited).unwrap();

    click_block_button(&mut rt, 1);
    finish_writing(&mut rt);

    assert_eq!(index_text(repo.path()), before);
    let notice = rt.state.notice.clone().unwrap_or_default();
    assert!(notice.contains("a changé depuis son affichage"), "{notice}");
}

#[test]
fn block_buttons_exist_only_for_tracked_text_files_in_the_unstaged_view() {
    let repo = three_block_repo();
    std::fs::write(repo.path().join("untracked.txt"), "a\nb\n").unwrap();
    let mut rt = blocks_runtime(&repo);
    let tracked = rt
        .state
        .files
        .iter()
        .position(|&i| rt.state.entries()[i].path == Path::new("f.txt"))
        .unwrap();
    let untracked = 1 - tracked;
    let untracked_index = rt.state.entry_at(untracked).unwrap().0;
    rt.update(move |s, tx| {
        s.jump_to(untracked);
        crate::keys::align_file(tx, untracked_index);
    });
    for _ in 0..6 {
        step(&mut rt);
    }
    let blocks_for_untracked = rt.elements().into_iter().any(|e| {
        e.key
            .as_deref()
            .is_some_and(|k| k.starts_with(&format!("hb{untracked_index}:")))
    });
    assert!(!blocks_for_untracked, "an untracked file is staged whole");

    rt.state.set_view(ViewKind::Staged);
    for _ in 0..6 {
        step(&mut rt);
    }
    assert!(
        !rt.elements()
            .into_iter()
            .any(|e| e.key.as_deref().is_some_and(|k| k.starts_with("hb")))
    );
}
