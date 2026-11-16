use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use rune::{Env, Runtime};
use tempfile::TempDir;

use crate::state::{Screen, State, ViewKind, path_from_args};
use crate::view::view;

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
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repo_with_files(count: usize) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    for i in 0..count {
        let path = dir.path().join(format!("src/dir{}/file_{i:04}.rs", i % 7));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            (0..12).map(|n| format!("line {n}\n")).collect::<String>(),
        )
        .unwrap();
    }
    std::fs::write(dir.path().join("staged.txt"), "a\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    for i in 0..count {
        let path = dir.path().join(format!("src/dir{}/file_{i:04}.rs", i % 7));
        let mut text = std::fs::read_to_string(&path).unwrap();
        text = text.replace("line 5\n", "changed 5\nextra\n");
        std::fs::write(path, text).unwrap();
    }
    std::fs::write(dir.path().join("staged.txt"), "a\nb\n").unwrap();
    git(dir.path(), &["add", "staged.txt"]);
    std::fs::write(dir.path().join("untracked.txt"), "u\n").unwrap();
    dir
}

fn opened(dir: &Path) -> State {
    let mut state = State::new(None);
    state.setup(Arc::new(|| {}));
    state.open_blocking(dir.to_path_buf(), Duration::from_secs(20));
    state
}

fn runtime(state: State) -> Runtime<State> {
    let mut rt = Runtime::new(
        state,
        view,
        (1280.0, 800.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    for _ in 0..40 {
        rt.frame(1.0 / 60.0);
        std::thread::sleep(Duration::from_millis(10));
        rt.state.drain();
    }
    rt.frame(1.0 / 60.0);
    rt
}

fn mounted(rt: &Runtime<State>, prefix: &str) -> usize {
    rt.elements()
        .into_iter()
        .filter(|e| e.key.as_deref().is_some_and(|k| k.starts_with(prefix)))
        .count()
}

#[test]
fn the_path_argument_opens_the_first_directory_and_ignores_flags_and_files() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file");
    std::fs::write(&file, "x").unwrap();
    let args =
        |list: &[&std::ffi::OsStr]| list.iter().map(|a| a.to_os_string()).collect::<Vec<_>>();
    let program = std::ffi::OsStr::new("gitpoulpe");
    let flag = std::ffi::OsStr::new("--snap");
    let missing = std::ffi::OsStr::new("/nonexistent-path");
    assert_eq!(
        path_from_args(args(&[
            program,
            flag,
            file.as_os_str(),
            missing,
            dir.path().as_os_str()
        ])),
        Some(dir.path().to_path_buf())
    );
    assert_eq!(path_from_args(args(&[program])), None);
}

#[test]
fn opening_lists_changed_files_per_view_and_marks_the_screen_ready() {
    let repo = repo_with_files(5);
    let state = opened(repo.path());
    assert_eq!(state.screen, Screen::Ready);
    assert_eq!(state.count(ViewKind::Unstaged), 6);
    assert_eq!(state.count(ViewKind::Staged), 1);
    assert_eq!(state.files.len(), 6);
    assert_eq!(
        state.repo.as_ref().unwrap().snapshot.branch.head.as_deref(),
        Some("main")
    );
}

#[test]
fn switching_view_and_filtering_narrow_the_file_list() {
    let repo = repo_with_files(5);
    let mut state = opened(repo.path());
    state.set_view(ViewKind::Staged);
    assert_eq!(state.files.len(), 1);
    state.set_view(ViewKind::Unstaged);
    state.filter.text = "FILE_0003".to_owned();
    state.refilter();
    assert_eq!(state.files.len(), 1);
    assert!(state.entry_at(0).unwrap().1.path.ends_with("file_0003.rs"));
    state.filter.text = "no-such-file".to_owned();
    state.refilter();
    assert!(state.files.is_empty());
    state.filter.text = "src/dir".to_owned();
    state.refilter();
    assert_eq!(state.files.len(), 5);
    assert!(state.files.iter().all(|&i| {
        state.entries()[i]
            .path
            .to_string_lossy()
            .starts_with("src/dir")
    }));
}

#[test]
fn failures_to_open_are_explicit_screens_not_an_empty_repository() {
    let not_a_repo = tempfile::tempdir().unwrap();
    let state = opened(not_a_repo.path());
    let Screen::Failed { message, .. } = &state.screen else {
        panic!("expected Failed, got {:?}", state.screen)
    };
    assert_eq!(message, "Ce dossier n'est pas un dépôt Git.");

    let state = opened(&not_a_repo.path().join("missing"));
    let Screen::Failed { message, .. } = &state.screen else {
        panic!("expected Failed")
    };
    assert_eq!(message, "Ce dossier n'existe pas.");

    let fresh = State::new(None);
    assert_eq!(fresh.screen, Screen::Welcome);
}

#[test]
fn one_continuous_scroll_covers_every_file_with_a_bounded_mounted_window() {
    let repo = repo_with_files(400);
    let state = opened(repo.path());
    assert_eq!(state.files.len(), 401);
    let rt = runtime(state);

    assert!(
        rt.state.layout.total >= 401 * 2,
        "layout rows: {}",
        rt.state.layout.total
    );
    let mounted_rows = mounted(&rt, "h") + mounted(&rt, "l");
    assert!(
        mounted_rows > 5 && mounted_rows < 400,
        "mounted rows: {mounted_rows}"
    );
    assert!(
        mounted(&rt, "fr") < 100,
        "mounted list rows: {}",
        mounted(&rt, "fr")
    );
    let loaded = rt.state.docs.values().filter(|d| d.meta.is_some()).count();
    assert!(
        loaded < 100,
        "documents opened ahead of the viewport: {loaded}"
    );
}

#[test]
fn headers_show_status_and_line_counts_once_the_document_is_known() {
    let repo = repo_with_files(3);
    let mut state = opened(repo.path());
    state.settle(3, Duration::from_secs(20));
    let (index, _) = state.entry_at(0).unwrap();
    let meta = state.doc(index).unwrap().meta.clone().unwrap();
    assert_eq!((meta.added, meta.removed), (2, 1));
    assert!(
        state
            .line(index, 0)
            .is_some_and(|l| l.text.starts_with(b"@@"))
    );
}
