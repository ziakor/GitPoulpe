use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use gitpoulpe_core::{DiffLineKind, DocumentKind};
use rune::prefs::Prefs;
use tempfile::TempDir;

use super::{git, keyboard_runtime, opened, press, repo_with_files, runtime, step};
use crate::state::{Screen, State, ViewKind};

fn git_output(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn git_may_fail(dir: &Path, args: &[&str]) {
    let _ = Command::new("git")
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
}

fn init(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
}

fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

fn paths_of(state: &State, view: ViewKind) -> Vec<String> {
    let mut paths: Vec<String> = state
        .entries()
        .iter()
        .filter(|e| match view {
            ViewKind::Unstaged => e.is_unstaged() || e.is_conflict(),
            ViewKind::Staged => e.is_staged(),
            ViewKind::Commit => false,
        })
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect();
    paths.sort();
    paths
}

fn lines_of(state: &State, path: &str) -> Vec<(DiffLineKind, String)> {
    let position = state
        .files
        .iter()
        .position(|&i| state.entries()[i].path == Path::new(path))
        .unwrap_or_else(|| panic!("{path} is not listed"));
    let index = state.files[position];
    let meta = state.doc(index).and_then(|d| d.meta.as_ref()).unwrap();
    (0..meta.lines)
        .filter_map(|n| state.line(index, n))
        .map(|l| (l.kind, String::from_utf8_lossy(&l.text).into_owned()))
        .collect()
}

fn shows_text(rt: &rune::Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

#[test]
fn paths_with_spaces_special_characters_and_invalid_utf8_open_to_the_same_state() {
    let parent = tempfile::tempdir().unwrap();
    let names: Vec<OsString> = vec![
        "plain".into(),
        "with space (1) #[x] é 日本".into(),
        OsString::from_vec(b"bytes-\xff\xfe-name".to_vec()),
    ];
    let mut expected = None;
    for name in names {
        let dir = parent.path().join(&name);
        std::fs::create_dir(&dir).unwrap();
        init(&dir);
        std::fs::write(dir.join("a.txt"), "1\n").unwrap();
        commit_all(&dir, "base");
        std::fs::write(dir.join("a.txt"), "2\n").unwrap();
        std::fs::write(dir.join("new.txt"), "n\n").unwrap();

        let by_picker = opened(&dir);
        assert_eq!(by_picker.screen, Screen::Ready, "{name:?}");
        let args = vec![OsString::from("gitpoulpe"), dir.clone().into_os_string()];
        let by_argument_path = crate::state::path_from_args(args).unwrap();
        assert_eq!(by_argument_path, dir, "{name:?}");
        let by_argument = opened(&by_argument_path);

        let summary = |s: &State| {
            (
                paths_of(s, ViewKind::Unstaged),
                paths_of(s, ViewKind::Staged),
                s.repo.as_ref().unwrap().snapshot.branch.head.clone(),
            )
        };
        assert_eq!(summary(&by_picker), summary(&by_argument), "{name:?}");
        match &expected {
            None => expected = Some(summary(&by_picker)),
            Some(first) => assert_eq!(&summary(&by_picker), first, "{name:?}"),
        }
    }
}

#[test]
fn worktrees_and_repositories_without_a_commit_are_inspectable_and_a_bare_one_says_why() {
    let main = tempfile::tempdir().unwrap();
    init(main.path());
    std::fs::write(main.path().join("a.txt"), "1\n").unwrap();
    commit_all(main.path(), "base");
    let linked_parent = tempfile::tempdir().unwrap();
    let linked = linked_parent.path().join("linked tree");
    git(
        main.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "other",
            linked.to_str().unwrap(),
        ],
    );
    std::fs::write(linked.join("a.txt"), "changed in the linked tree\n").unwrap();
    let state = opened(&linked);
    assert_eq!(state.screen, Screen::Ready);
    assert_eq!(paths_of(&state, ViewKind::Unstaged), vec!["a.txt"]);
    assert_eq!(
        state.repo.as_ref().unwrap().snapshot.branch.head.as_deref(),
        Some("other")
    );

    let fresh = tempfile::tempdir().unwrap();
    init(fresh.path());
    std::fs::write(fresh.path().join("first.txt"), "x\n").unwrap();
    git(fresh.path(), &["add", "first.txt"]);
    std::fs::write(fresh.path().join("loose.txt"), "y\n").unwrap();
    let state = opened(fresh.path());
    assert_eq!(state.screen, Screen::Ready);
    assert_eq!(paths_of(&state, ViewKind::Staged), vec!["first.txt"]);
    assert_eq!(paths_of(&state, ViewKind::Unstaged), vec!["loose.txt"]);

    let bare = tempfile::tempdir().unwrap();
    git(bare.path(), &["init", "-q", "--bare", "-b", "main"]);
    let state = opened(bare.path());
    let Screen::Failed { message, .. } = &state.screen else {
        panic!(
            "a bare repository has nothing to inspect: {:?}",
            state.screen
        )
    };
    assert!(
        message.contains("bare") && message.contains("répertoire de travail"),
        "{message}"
    );
}

#[test]
fn an_invalid_or_denied_folder_leaves_a_useful_message_and_another_repository_still_opens() {
    let repo = repo_with_files(3);
    let scratch = tempfile::tempdir().unwrap();
    let locked = scratch.path().join("locked");
    std::fs::create_dir_all(locked.join("inside")).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

    let mut state = State::new(None, Prefs::memory());
    state.setup(std::sync::Arc::new(|| {}));
    let message_for = |state: &mut State, path: PathBuf| {
        state.open_blocking(path, Duration::from_secs(20));
        let Screen::Failed { message, .. } = &state.screen else {
            panic!("expected a failure, got {:?}", state.screen)
        };
        message.clone()
    };
    let denied = message_for(&mut state, locked.join("inside"));
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(denied, "L'accès à ce dossier est refusé.");
    assert_eq!(
        message_for(&mut state, scratch.path().join("absent")),
        "Ce dossier n'existe pas."
    );
    assert_eq!(
        message_for(&mut state, scratch.path().to_path_buf()),
        "Ce dossier n'est pas un dépôt Git."
    );

    state.open_blocking(repo.path().to_path_buf(), Duration::from_secs(20));
    assert_eq!(state.screen, Screen::Ready);
    assert_eq!(state.count(ViewKind::Unstaged), 4);
}

#[test]
fn every_kind_of_change_is_listed_in_the_view_git_reports_it_in() {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path();
    init(dir);
    for name in [
        "modified.txt",
        "deleted.txt",
        "renamed.txt",
        "both.txt",
        "untouched.txt",
    ] {
        std::fs::write(dir.join(name), format!("{name}\nline\n")).unwrap();
    }
    commit_all(dir, "base");
    std::fs::write(dir.join("modified.txt"), "changed\n").unwrap();
    std::fs::remove_file(dir.join("deleted.txt")).unwrap();
    git(dir, &["mv", "renamed.txt", "renamed-to.txt"]);
    std::fs::write(dir.join("added.txt"), "new\n").unwrap();
    git(dir, &["add", "added.txt"]);
    std::fs::write(dir.join("both.txt"), "staged\n").unwrap();
    git(dir, &["add", "both.txt"]);
    std::fs::write(dir.join("both.txt"), "staged\nand then edited\n").unwrap();
    std::fs::write(dir.join("untracked.txt"), "u\n").unwrap();

    let state = opened(dir);
    let sorted = |text: String| {
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        lines.sort();
        lines
    };
    let mut unstaged = sorted(git_output(dir, &["diff", "--name-only"]));
    unstaged.extend(sorted(git_output(
        dir,
        &["ls-files", "--others", "--exclude-standard"],
    )));
    unstaged.sort();
    let staged = sorted(git_output(dir, &["diff", "--cached", "--name-only"]));

    assert_eq!(paths_of(&state, ViewKind::Unstaged), unstaged);
    assert_eq!(paths_of(&state, ViewKind::Staged), staged);
    let both = |view| paths_of(&state, view).contains(&"both.txt".to_owned());
    assert!(both(ViewKind::Unstaged) && both(ViewKind::Staged));
}

#[test]
fn a_clean_repository_loading_and_an_error_are_three_different_screens() {
    let clean = tempfile::tempdir().unwrap();
    init(clean.path());
    std::fs::write(clean.path().join("a.txt"), "1\n").unwrap();
    commit_all(clean.path(), "base");
    let rt = runtime(opened(clean.path()));
    assert!(shows_text(&rt, "Aucune modification"));
    assert!(!shows_text(&rt, "Ouverture…") && !shows_text(&rt, "Impossible d'ouvrir"));

    let mut loading = State::new(None, Prefs::memory());
    loading.screen = Screen::Opening(clean.path().to_path_buf());
    let rt = runtime(loading);
    assert!(shows_text(&rt, "Ouverture…"));
    assert!(!shows_text(&rt, "Aucune modification"));

    let scratch = tempfile::tempdir().unwrap();
    let rt = runtime(opened(scratch.path()));
    assert!(shows_text(&rt, "Impossible d'ouvrir ce dépôt"));
    assert!(!shows_text(&rt, "Aucune modification"));
}

#[test]
fn a_file_changed_in_the_index_and_in_the_working_directory_has_a_different_diff_in_each_view() {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path();
    init(dir);
    std::fs::write(dir.join("f.txt"), "head\n").unwrap();
    commit_all(dir, "base");
    std::fs::write(dir.join("f.txt"), "index\n").unwrap();
    git(dir, &["add", "f.txt"]);
    std::fs::write(dir.join("f.txt"), "worktree\n").unwrap();

    let mut state = opened(dir);
    state.settle(10, Duration::from_secs(20));
    let unstaged = lines_of(&state, "f.txt");
    state.set_view(ViewKind::Staged);
    state.settle(10, Duration::from_secs(20));
    let staged = lines_of(&state, "f.txt");

    let changes = |lines: &[(DiffLineKind, String)], kind: DiffLineKind| {
        lines
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, t)| t.trim_end().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(changes(&staged, DiffLineKind::Removed), ["head"]);
    assert_eq!(changes(&staged, DiffLineKind::Added), ["index"]);
    assert_eq!(changes(&unstaged, DiffLineKind::Removed), ["index"]);
    assert_eq!(changes(&unstaged, DiffLineKind::Added), ["worktree"]);

    let fresh = tempfile::tempdir().unwrap();
    init(fresh.path());
    std::fs::write(fresh.path().join("first.txt"), "one\n").unwrap();
    git(fresh.path(), &["add", "first.txt"]);
    let mut state = opened(fresh.path());
    state.set_view(ViewKind::Staged);
    state.settle(10, Duration::from_secs(20));
    assert_eq!(
        lines_of(&state, "first.txt")
            .iter()
            .filter(|(k, _)| *k == DiffLineKind::Added)
            .count(),
        1
    );
}

#[test]
fn the_interface_never_offers_an_action_that_changes_the_repository() {
    let (_repo, rt) = keyboard_runtime(5);
    let forbidden = [
        "ajouter",
        "valider",
        "supprimer",
        "annuler",
        "restaurer",
        "écraser",
        "stage",
        "push",
    ];
    for element in rt.elements() {
        if element.role != "button" {
            continue;
        }
        let label = element.label.unwrap_or_default().to_lowercase();
        assert!(
            !forbidden.iter().any(|word| label.contains(word)),
            "unexpected action: {label}"
        );
    }
}

fn many_changes(count: usize) -> TempDir {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path();
    init(dir);
    for i in 0..count {
        let path = dir.join(format!("pkg{:02}/mod_{:02}/file_{i:04}.rs", i % 40, i % 13));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "a\nb\nc\n").unwrap();
    }
    commit_all(dir, "base");
    for i in 0..count {
        let path = dir.join(format!("pkg{:02}/mod_{:02}/file_{i:04}.rs", i % 40, i % 13));
        std::fs::write(path, "a\nB\nc\n").unwrap();
    }
    repo
}

#[test]
fn filtering_three_thousand_two_hundred_changes_by_name_and_path_is_exact() {
    let repo = many_changes(3200);
    let mut state = opened(repo.path());
    assert_eq!(state.files.len(), 3200);

    let expect = |state: &mut State, text: &str, expected: usize| {
        state.filter.text = text.to_owned();
        state.filter_changed();
        assert_eq!(state.files.len(), expected, "filter {text:?}");
    };
    expect(&mut state, "file_0031", 1);
    expect(&mut state, "FILE_003", 10);
    let in_pkg = (0..3200).filter(|i| i % 40 == 7).count();
    expect(&mut state, "pkg07/", in_pkg);
    let in_both = (0..3200).filter(|i| i % 40 == 7 && i % 13 == 7).count();
    expect(&mut state, "pkg07/mod_07", in_both);
    expect(&mut state, "", 3200);
    expect(&mut state, "does-not-exist", 0);
    expect(&mut state, "  ", 3200);
}

#[test]
fn no_result_is_not_a_clean_view_and_clearing_the_filter_brings_the_files_back() {
    let repo = repo_with_files(4);
    let mut rt = runtime(opened(repo.path()));
    rt.state.filter.text = "does-not-exist".to_owned();
    rt.state.filter_changed();
    step(&mut rt);
    assert!(shows_text(&rt, "Aucun fichier ne correspond"));
    assert!(!shows_text(&rt, "Aucune modification"));

    rt.state.filter.text.clear();
    rt.state.filter_changed();
    step(&mut rt);
    assert!(!shows_text(&rt, "Aucun fichier ne correspond"));
    assert_eq!(rt.state.files.len(), 5);

    press(&mut rt, rune::Key::Text("2".to_owned()));
    rt.state.filter.text.clear();
    let clean_staged = repo_with_files(0);
    drop(clean_staged);
}

#[test]
fn the_keyboard_stays_usable_and_a_filter_change_mounts_no_extra_diff_on_a_long_list() {
    let repo = many_changes(3200);
    let mut rt = runtime(opened(repo.path()));
    rt.on_key = Some(crate::keys::on_key);
    for _ in 0..5 {
        press(&mut rt, rune::Key::Text("j".to_owned()));
    }
    assert_eq!(rt.state.current, Some(5));

    rt.state.filter.text = "pkg07/".to_owned();
    rt.state.filter_changed();
    for _ in 0..4 {
        step(&mut rt);
    }
    let expected = (0..3200).filter(|i| i % 40 == 7).count();
    assert_eq!(rt.state.files.len(), expected);
    assert!(
        rt.state.docs.len() < 150,
        "only the documents near the viewport are tracked: {}",
        rt.state.docs.len()
    );
    press(&mut rt, rune::Key::Text("j".to_owned()));
    assert!(rt.state.current.is_some());
}

#[test]
fn one_scroll_crosses_three_thousand_two_hundred_files_and_a_chosen_file_reaches_its_header() {
    let repo = many_changes(3200);
    let mut rt = runtime(opened(repo.path()));
    rt.on_key = Some(crate::keys::on_key);
    press(&mut rt, rune::Key::End);
    for _ in 0..15 {
        step(&mut rt);
    }
    assert_eq!(rt.state.current, Some(3199));
    assert!(
        super::mounted(&rt, "t") < 400,
        "the mounted rows stay bounded: {}",
        super::mounted(&rt, "t")
    );

    press(&mut rt, rune::Key::Home);
    for _ in 0..6 {
        step(&mut rt);
    }
    rt.update(|s, tx| {
        s.jump_to(1600);
        crate::keys::align_file(tx, s.files[1600]);
    });
    for _ in 0..8 {
        step(&mut rt);
    }
    let header = format!("h{}", rt.state.files[1600]);
    let rect = super::rect_of(&rt, &header).expect("the chosen header is mounted");
    let top = super::rect_of(&rt, "viewport").map_or(0.0, |r| r[1]);
    assert!(
        (rect[1] - top).abs() < 40.0,
        "header {rect:?} near the top {top}"
    );
}

#[test]
fn binary_conflict_and_unreadable_files_state_what_they_are_where_they_sit_in_the_stream() {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path();
    init(dir);
    std::fs::write(dir.join("a_text.txt"), "1\n").unwrap();
    std::fs::write(dir.join("b_binary.dat"), b"\0old").unwrap();
    std::fs::write(dir.join("c_conflict.txt"), "base\n").unwrap();
    std::fs::write(dir.join("d_locked.txt"), "1\n").unwrap();
    commit_all(dir, "base");
    git(dir, &["checkout", "-q", "-b", "side"]);
    std::fs::write(dir.join("c_conflict.txt"), "side\n").unwrap();
    commit_all(dir, "side");
    git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("c_conflict.txt"), "main\n").unwrap();
    commit_all(dir, "main");
    git_may_fail(dir, &["merge", "side"]);
    std::fs::write(dir.join("a_text.txt"), "2\n").unwrap();
    std::fs::write(dir.join("b_binary.dat"), b"\0new").unwrap();
    std::fs::write(dir.join("d_locked.txt"), "2\n").unwrap();
    std::fs::set_permissions(
        dir.join("d_locked.txt"),
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();

    let mut rt = runtime(opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    rt.state.settle(10, Duration::from_secs(20));
    let kinds: Vec<_> = rt
        .state
        .files
        .iter()
        .map(|&i| {
            (
                rt.state.entries()[i].path.to_string_lossy().into_owned(),
                rt.state
                    .doc(i)
                    .and_then(|d| d.meta.as_ref())
                    .and_then(|m| m.kind.clone()),
            )
        })
        .collect();
    let kind = |name: &str| kinds.iter().find(|(p, _)| p == name).unwrap().1.clone();
    assert_eq!(kind("b_binary.dat"), Some(DocumentKind::Binary));
    assert_eq!(kind("c_conflict.txt"), Some(DocumentKind::Conflict));
    assert!(matches!(
        kind("d_locked.txt"),
        Some(DocumentKind::Unreadable(_))
    ));

    for _ in 0..6 {
        step(&mut rt);
    }
    for (needle, name) in [
        ("Fichier binaire", "b_binary.dat"),
        ("Conflit de fusion", "c_conflict.txt"),
        ("Lecture impossible", "d_locked.txt"),
    ] {
        let position = rt
            .state
            .files
            .iter()
            .position(|&i| rt.state.entries()[i].path == Path::new(name))
            .unwrap();
        rt.update(move |s, tx| {
            s.jump_to(position);
            crate::keys::align_file(tx, s.files[position]);
        });
        for _ in 0..8 {
            step(&mut rt);
        }
        assert!(shows_text(&rt, needle), "{name} shows {needle:?}");
    }
    std::fs::set_permissions(
        dir.join("d_locked.txt"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
}

#[test]
fn a_selection_across_unmounted_lines_of_a_fifty_thousand_line_file_is_copied_and_rendering_stays_bounded()
 {
    let repo = tempfile::tempdir().unwrap();
    init(repo.path());
    std::fs::write(repo.path().join("seed.txt"), "s\n").unwrap();
    commit_all(repo.path(), "base");
    std::fs::write(
        repo.path().join("big.txt"),
        (0..50_000)
            .map(|i| format!("row {i}\n"))
            .collect::<String>(),
    )
    .unwrap();

    let mut rt = runtime(opened(repo.path()));
    rt.on_key = Some(crate::keys::on_key);
    rt.state.settle(1, Duration::from_secs(60));
    let big = rt
        .state
        .files
        .iter()
        .position(|&i| rt.state.entries()[i].path == Path::new("big.txt"))
        .unwrap();
    let first = super::anchor(&rt.state, big, 10, 0);
    let last = super::anchor(&rt.state, big, 40_000, 0);
    assert!(rt.state.start_copy(first, last, None));
    let (text, lines, truncated) = super::expect_copy(&mut rt.state);
    assert!(!truncated);
    assert!((39_980..=40_020).contains(&lines), "{lines} lines");
    assert!(
        text.contains("row 25000"),
        "a line that was never mounted is in the copy"
    );

    press(&mut rt, rune::Key::End);
    for _ in 0..12 {
        step(&mut rt);
    }
    let mounted = super::mounted(&rt, "l") + super::mounted(&rt, "t");
    assert!(
        (1..400).contains(&mounted),
        "rendered rows stay bounded: {mounted}"
    );
    assert!(rt.state.loaded_lines() <= 16 * crate::state::CHUNK_LINES as usize);
}
