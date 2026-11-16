use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use rune::prefs::Prefs;
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
    let mut state = State::new(None, Prefs::memory());
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

    let fresh = State::new(None, Prefs::memory());
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

#[test]
fn reaching_the_end_of_the_scroll_makes_the_last_file_current() {
    let repo = repo_with_files(30);
    let mut state = opened(repo.path());
    let total = state.layout.total as usize;
    let last = state.files.len() - 1;
    state.on_visible(0..8);
    assert_eq!(state.current, Some(0));
    state.on_visible(total - 6..total);
    assert_eq!(state.current, Some(last));
}

#[test]
fn a_file_chosen_in_the_list_stays_current_while_it_is_on_screen() {
    let repo = repo_with_files(30);
    let mut state = opened(repo.path());
    let total = state.layout.total as usize;
    let last = state.files.len() - 1;
    state.jump_to(last - 1);
    state.on_visible(total - 6..total);
    assert_eq!(
        state.current,
        Some(last - 1),
        "the explicit choice wins at the end of the scroll"
    );
    state.on_visible(0..8);
    assert_eq!(state.current, Some(0), "scrolling away releases the choice");
    state.on_visible(total - 6..total);
    assert_eq!(state.current, Some(last));
}

#[test]
fn the_list_width_is_clamped_remembered_and_resettable() {
    let mut state = State::new(None, Prefs::memory());
    assert_eq!(state.list_width, crate::state::LIST_WIDTH_DEFAULT);
    state.resize_list(10.0);
    assert_eq!(state.list_width, crate::state::LIST_WIDTH_MIN);
    state.resize_list(5000.0);
    assert_eq!(state.list_width, crate::state::LIST_WIDTH_MAX);
    state.resize_list(f32::NAN);
    assert_eq!(state.list_width, crate::state::LIST_WIDTH_DEFAULT);

    let mut prefs = Prefs::memory();
    prefs.set("list_width", &500.0f32).unwrap();
    assert_eq!(State::new(None, prefs).list_width, 500.0);
    let mut prefs = Prefs::memory();
    prefs.set("list_width", &9000.0f32).unwrap();
    assert_eq!(
        State::new(None, prefs).list_width,
        crate::state::LIST_WIDTH_MAX
    );
}

#[test]
fn dragging_the_divider_resizes_the_list_and_a_double_click_resets_it() {
    let repo = repo_with_files(5);
    let mut rt = runtime(opened(repo.path()));
    let start = rt.state.list_width;
    let (x, y) = rt.find("divider").expect("the divider is on screen");

    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_move(x + 80.0, y);
    rt.pointer_move(x + 120.0, y);
    rt.pointer_up(x + 120.0, y);
    rt.frame(1.0 / 60.0);
    assert!(
        (rt.state.list_width - (start + 120.0)).abs() < 1.5,
        "width after drag: {}",
        rt.state.list_width
    );

    let (x, y) = rt.find("divider").unwrap();
    for _ in 0..2 {
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.05);
    }
    assert_eq!(rt.state.list_width, crate::state::LIST_WIDTH_DEFAULT);
}

#[test]
fn the_divider_can_be_resized_from_the_keyboard() {
    let repo = repo_with_files(5);
    let mut rt = runtime(opened(repo.path()));
    let start = rt.state.list_width;
    rt.update(|_, tx| tx.focus("divider", true));
    rt.frame(1.0 / 60.0);
    rt.key(rune::KeyEvent {
        key: rune::Key::Right,
        mods: rune::Mods::default(),
    });
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.list_width, start + 16.0);
    rt.key(rune::KeyEvent {
        key: rune::Key::Left,
        mods: rune::Mods {
            shift: true,
            ..Default::default()
        },
    });
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.list_width, start - 48.0);
}

#[test]
fn the_current_file_is_the_one_under_the_reading_line_not_the_first_one_on_screen() {
    let repo = repo_with_files(30);
    let mut state = opened(repo.path());
    let start = state.layout.offsets[5] as usize + 1;
    let rows = start..start + 40;
    let (first_file, _) = state.locate(start as u64).unwrap();
    let reading_row = start + 40 * crate::state::READING_LINE_PERCENT / 100;
    let (reading_file, _) = state.locate(reading_row as u64).unwrap();
    assert_ne!(
        first_file, reading_file,
        "the fixture must separate the two rules"
    );
    state.on_visible(rows);
    assert_eq!(state.current, Some(reading_file));
}

#[test]
fn the_first_file_is_current_at_the_very_top_and_the_last_at_the_very_bottom() {
    let repo = repo_with_files(30);
    let mut state = opened(repo.path());
    let total = state.layout.total as usize;
    state.on_visible(0..60);
    assert_eq!(state.current, Some(0));
    state.on_visible(total - 60..total);
    assert_eq!(state.current, Some(state.files.len() - 1));
}

#[test]
fn a_chosen_file_is_released_once_it_is_no_longer_the_first_on_screen() {
    let repo = repo_with_files(30);
    let mut state = opened(repo.path());
    state.settle(state.files.len(), Duration::from_secs(30));
    assert!(
        state.layout.total > 31 * 8,
        "every document is loaded: {}",
        state.layout.total
    );
    let target = 10;
    let top = state.layout.offsets[target] as usize;
    state.jump_to(target);
    state.on_visible(top..top + 40);
    assert_eq!(state.current, Some(target));
    let moved = state.layout.offsets[target] as usize + 30;
    state.on_visible(moved..moved + 40);
    assert_ne!(
        state.current,
        Some(target),
        "scrolling past the chosen file releases it"
    );
    assert_eq!(state.pinned, None);
}
