use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use rune::prefs::Prefs;
use rune::{Env, Runtime};
use tempfile::TempDir;

mod acceptance;
mod amending;
mod branching;
mod cloning;
mod committing;
mod conflicting;
mod history;
mod ignoring;
mod initializing;
mod inspecting;
mod merging;
mod profiles;
mod publishing;
mod pulling;
mod rebasing;
mod reflogging;
mod remoting;
mod resetting;
mod sequencing;
mod staging;
mod stashing;
mod tagging;
mod undoing;
mod worktreeing;

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

fn rect_of(rt: &Runtime<State>, key: &str) -> Option<[f32; 4]> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .map(|e| e.rect)
}

fn step(rt: &mut Runtime<State>) {
    for _ in 0..10 {
        rt.state.drain();
        rt.update(|s, tx| {
            s.apply_follow(tx);
            s.apply_copy(tx);
            s.apply_anchor(tx);
            s.apply_reveal(tx);
        });
        rt.frame(1.0 / 60.0);
        std::thread::sleep(Duration::from_millis(3));
    }
}

fn current_row_is_inside_the_list(rt: &Runtime<State>) -> bool {
    let Some(position) = rt.state.current else {
        return false;
    };
    let (index, _) = rt.state.entry_at(position).unwrap();
    let Some(viewport) = rect_of(rt, "list-viewport") else {
        return false;
    };
    let Some(row) = rect_of(rt, &format!("f{index}")) else {
        return false;
    };
    row[1] >= viewport[1] - 1.0 && row[1] + row[3] <= viewport[1] + viewport[3] + 1.0
}

#[test]
fn the_file_list_scrolls_to_keep_the_current_file_in_view_both_ways() {
    let repo = repo_with_files(80);
    let mut rt = runtime(opened(repo.path()));
    step(&mut rt);
    assert!(current_row_is_inside_the_list(&rt));
    let list_rows = rt.state.files.len();

    let mut moved_past_the_first_screen = false;
    for _ in 0..16 {
        rt.wheel(900.0, 300.0, -2200.0);
        step(&mut rt);
        assert!(
            current_row_is_inside_the_list(&rt),
            "current={:?} of {list_rows}",
            rt.state.current
        );
        moved_past_the_first_screen |= rt.state.current.is_some_and(|c| c > 20);
    }
    assert!(
        moved_past_the_first_screen,
        "the fixture must scroll the diff past the first list screen"
    );

    for _ in 0..16 {
        rt.wheel(900.0, 300.0, 2200.0);
        step(&mut rt);
        assert!(
            current_row_is_inside_the_list(&rt),
            "going back up, current={:?}",
            rt.state.current
        );
    }
    assert!(rt.state.current.is_some_and(|c| c < 5));
}

fn keyboard_runtime(files: usize) -> (TempDir, Runtime<State>) {
    let repo = repo_with_files(files);
    let mut rt = runtime(opened(repo.path()));
    rt.on_key = Some(crate::keys::on_key);
    (repo, rt)
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused)
        .and_then(|e| e.key)
}

fn press(rt: &mut Runtime<State>, key: rune::Key) {
    rt.key(rune::KeyEvent {
        key,
        mods: rune::Mods::default(),
    });
    step(rt);
}

fn type_text(rt: &mut Runtime<State>, text: &str) {
    press(rt, rune::Key::Text(text.to_owned()));
}

fn focus(rt: &mut Runtime<State>, key: &str) {
    let key = key.to_owned();
    rt.update(move |_, tx| tx.focus(key, true));
    step(rt);
}

#[test]
fn slash_focuses_the_filter_and_escape_clears_then_leaves_it() {
    let (_repo, mut rt) = keyboard_runtime(10);
    assert_eq!(focused(&rt), None);
    type_text(&mut rt, "/");
    assert_eq!(focused(&rt).as_deref(), Some("filter"));
    assert_eq!(
        rt.state.filter.text, "",
        "the slash opens the filter, it is not typed into it"
    );
    assert!(
        rt.elements().iter().any(|e| e.focused && e.focus_visible),
        "the focus ring is visible"
    );

    type_text(&mut rt, "file_0003");
    assert_eq!(rt.state.filter.text, "file_0003");
    assert_eq!(rt.state.files.len(), 1);

    press(&mut rt, rune::Key::Escape);
    assert_eq!(rt.state.filter.text, "");
    assert_eq!(
        rt.state.files.len(),
        11,
        "the first Escape clears the filter"
    );
    assert_eq!(focused(&rt).as_deref(), Some("filter"));

    press(&mut rt, rune::Key::Escape);
    assert_eq!(focused(&rt), None, "the second Escape leaves the field");
}

#[test]
fn enter_in_the_filter_jumps_to_the_first_result() {
    let (_repo, mut rt) = keyboard_runtime(10);
    type_text(&mut rt, "/");
    type_text(&mut rt, "file_0006");
    press(&mut rt, rune::Key::Enter);
    assert_eq!(rt.state.current, Some(0));
    assert_eq!(rt.state.pinned, Some(0));
    assert_eq!(focused(&rt), None);
}

#[test]
fn j_and_k_step_through_the_files_and_stop_at_the_ends() {
    let (_repo, mut rt) = keyboard_runtime(10);
    let last = rt.state.files.len() - 1;
    type_text(&mut rt, "j");
    assert_eq!(rt.state.current, Some(1));
    type_text(&mut rt, "j");
    assert_eq!(rt.state.current, Some(2));
    type_text(&mut rt, "k");
    type_text(&mut rt, "k");
    type_text(&mut rt, "k");
    assert_eq!(rt.state.current, Some(0));
    for _ in 0..(last + 3) {
        type_text(&mut rt, "j");
    }
    assert_eq!(rt.state.current, Some(last));
}

#[test]
fn digits_switch_between_the_two_views() {
    let (_repo, mut rt) = keyboard_runtime(5);
    type_text(&mut rt, "2");
    assert_eq!(rt.state.view, ViewKind::Staged);
    type_text(&mut rt, "1");
    assert_eq!(rt.state.view, ViewKind::Unstaged);
}

#[test]
fn shortcuts_never_fire_while_typing_in_the_filter() {
    let (_repo, mut rt) = keyboard_runtime(5);
    type_text(&mut rt, "/");
    type_text(&mut rt, "j1k2/");
    assert_eq!(rt.state.filter.text, "j1k2/");
    assert_eq!(rt.state.view, ViewKind::Unstaged);
    assert_eq!(rt.state.pinned, None);
}

#[test]
fn the_tabs_are_one_tab_stop_and_arrows_move_between_them() {
    let (_repo, mut rt) = keyboard_runtime(5);
    focus(&mut rt, "Non indexés");
    assert_eq!(focused(&rt).as_deref(), Some("Non indexés"));
    press(&mut rt, rune::Key::Right);
    assert_eq!(focused(&rt).as_deref(), Some("Indexés"));
    type_text(&mut rt, " ");
    assert_eq!(
        rt.state.view,
        ViewKind::Staged,
        "Space presses the focused tab"
    );
    press(&mut rt, rune::Key::Left);
    assert_eq!(focused(&rt).as_deref(), Some("Non indexés"));
}

#[test]
fn the_file_list_is_navigated_with_arrows_and_entered_with_return() {
    let (_repo, mut rt) = keyboard_runtime(10);
    let (first, _) = rt.state.entry_at(0).unwrap();
    let (second, _) = rt.state.entry_at(1).unwrap();
    let (third, _) = rt.state.entry_at(2).unwrap();
    focus(&mut rt, &format!("f{first}"));
    assert_eq!(focused(&rt), Some(format!("f{first}")));
    press(&mut rt, rune::Key::Down);
    assert_eq!(focused(&rt), Some(format!("f{second}")));
    press(&mut rt, rune::Key::Down);
    assert_eq!(focused(&rt), Some(format!("f{third}")));
    press(&mut rt, rune::Key::Enter);
    assert_eq!(rt.state.current, Some(2));
    assert_eq!(rt.state.pinned, Some(2));
    press(&mut rt, rune::Key::Up);
    assert_eq!(focused(&rt), Some(format!("f{second}")));
}

#[test]
fn page_keys_home_and_arrows_scroll_the_diff_from_the_keyboard() {
    let (_repo, mut rt) = keyboard_runtime(60);
    let start = rt.state.visible.start;
    press(&mut rt, rune::Key::PageDown);
    let after_page = rt.state.visible.start;
    assert!(
        after_page > start + 10,
        "a page moves many rows: {start} -> {after_page}"
    );
    press(&mut rt, rune::Key::Down);
    let after_line = rt.state.visible.start;
    assert!(
        after_line > after_page && after_line - after_page <= 5,
        "a line key moves a few rows"
    );
    press(&mut rt, rune::Key::End);
    for _ in 0..6 {
        step(&mut rt);
    }
    assert!(
        rt.state.visible.end as u64 >= rt.state.layout.total,
        "End reaches the end: visible={:?} total={}",
        rt.state.visible,
        rt.state.layout.total
    );
    assert_eq!(rt.state.current, Some(rt.state.files.len() - 1));
    press(&mut rt, rune::Key::Home);
    assert_eq!(rt.state.visible.start, 0);
    assert_eq!(rt.state.current, Some(0));
}

#[test]
fn every_focusable_control_shows_a_visible_focus_when_reached_by_tab() {
    let (_repo, mut rt) = keyboard_runtime(5);
    let mut visited = std::collections::BTreeSet::new();
    for _ in 0..20 {
        press(&mut rt, rune::Key::Tab);
        if let Some(info) = rt.elements().into_iter().find(|e| e.focused) {
            assert!(info.focus_visible, "no visible focus on {:?}", info.key);
            visited.insert(info.key.unwrap_or_default());
        }
    }
    for expected in ["open", "theme", "filter", "viewport", "divider"] {
        assert!(
            visited.contains(expected),
            "Tab never reached {expected}: {visited:?}"
        );
    }
}

#[test]
fn the_end_key_stays_at_the_true_end_while_the_last_files_load_and_is_released_by_scrolling_up() {
    let (_repo, mut rt) = keyboard_runtime(60);
    press(&mut rt, rune::Key::End);
    for _ in 0..12 {
        step(&mut rt);
    }
    assert!(rt.state.stick_end);
    assert!(
        rt.state.visible.end as u64 >= rt.state.layout.total,
        "still at the end after the last documents loaded: visible={:?} total={}",
        rt.state.visible,
        rt.state.layout.total
    );

    rt.wheel(900.0, 300.0, 1500.0);
    for _ in 0..8 {
        step(&mut rt);
    }
    assert!(!rt.state.stick_end, "scrolling up releases the end");
    let position = rt.state.visible.start;
    for _ in 0..6 {
        step(&mut rt);
    }
    assert!(
        rt.state.visible.start <= position + 1,
        "the view no longer snaps back to the end"
    );
}

fn settled(files: usize) -> (TempDir, State) {
    let repo = repo_with_files(files);
    let mut state = opened(repo.path());
    state.settle(state.files.len(), Duration::from_secs(30));
    (repo, state)
}

#[test]
fn split_mode_counts_one_row_per_pairing_instead_of_one_per_line() {
    let (_repo, mut state) = settled(4);
    let (index, _) = state.entry_at(0).unwrap();
    let unified = state.body_rows(index);
    state.set_mode(crate::state::DiffMode::Split);
    let split = state.body_rows(index);
    assert_eq!(
        unified, 10,
        "header, 3 context, 1 removed, 2 added, 3 context"
    );
    assert_eq!(
        split, 9,
        "the removed line shares a row with the first added line"
    );
    assert_eq!(
        state.layout.total,
        state
            .files
            .iter()
            .map(|&i| 1 + state.body_rows(i))
            .sum::<u64>()
    );
}

#[test]
fn the_saved_mode_is_restored_and_an_unknown_one_falls_back_to_unified() {
    let mut prefs = Prefs::memory();
    prefs.set("diff_mode", "split").unwrap();
    assert_eq!(State::new(None, prefs).mode, crate::state::DiffMode::Split);
    let mut prefs = Prefs::memory();
    prefs.set("diff_mode", "sideways").unwrap();
    assert_eq!(
        State::new(None, prefs).mode,
        crate::state::DiffMode::Unified
    );
    assert_eq!(
        State::new(None, Prefs::memory()).mode,
        crate::state::DiffMode::Unified
    );
}

#[test]
fn the_m_key_and_the_toggle_buttons_switch_modes_both_ways() {
    let (_repo, mut rt) = keyboard_runtime(6);
    type_text(&mut rt, "m");
    assert_eq!(rt.state.mode, crate::state::DiffMode::Split);
    type_text(&mut rt, "m");
    assert_eq!(rt.state.mode, crate::state::DiffMode::Unified);

    let (x, y) = rect_of(&rt, "mode-split")
        .map(|r| (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0))
        .unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    step(&mut rt);
    assert_eq!(rt.state.mode, crate::state::DiffMode::Split);
    let (x, y) = rect_of(&rt, "mode-unified")
        .map(|r| (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0))
        .unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    step(&mut rt);
    assert_eq!(rt.state.mode, crate::state::DiffMode::Unified);
}

#[test]
fn switching_mode_keeps_the_reading_position_on_the_same_context_line() {
    let (_repo, state) = settled(30);
    let mut rt = runtime(state);
    step(&mut rt);
    let position = 12;
    let (index, _) = rt.state.entry_at(position).unwrap();
    let context_line = 2;
    let target_row = rt.state.layout.offsets[position] + 1 + context_line;
    let view_rows = 30usize;
    let start = target_row as usize - view_rows * crate::state::READING_LINE_PERCENT / 100;
    rt.state.visible = start..start + view_rows;
    assert_eq!(
        rt.state.reading_anchor(),
        Some((position, Some(context_line)))
    );

    rt.update(|s, tx| s.toggle_mode(tx));
    step(&mut rt);
    let split = rt
        .state
        .split_index(index)
        .expect("the document has a split index")
        .clone();
    let expected_row =
        rt.state.layout.offsets[position] + 1 + split.row_of_line(context_line).unwrap();
    let top = rt.state.visible.start as u64;
    assert!(
        top.abs_diff(expected_row) <= 1,
        "top row {top}, expected {expected_row}"
    );
    assert!(
        rt.state.layout.offsets[position + 1] > expected_row,
        "the anchor stays inside the same file"
    );
}

#[test]
fn split_rows_render_two_cells_and_the_row_count_follows_the_mode() {
    let (_repo, state) = settled(6);
    let (index, _) = state.entry_at(0).unwrap();
    let mut rt = runtime(state);
    step(&mut rt);
    let mounted_rows = |rt: &Runtime<State>| -> Vec<String> {
        rt.elements()
            .into_iter()
            .filter_map(|e| e.key)
            .filter(|k| k.starts_with(&format!("l{index}:")))
            .collect()
    };
    assert!(
        mounted_rows(&rt).contains(&format!("l{index}:9")),
        "unified has ten lines (0..=9)"
    );
    rt.update(|s, tx| s.toggle_mode(tx));
    for _ in 0..12 {
        step(&mut rt);
    }
    let rows = mounted_rows(&rt);
    assert!(rows.contains(&format!("l{index}:8")));
    assert!(
        !rows.contains(&format!("l{index}:9")),
        "split has nine rows (0..=8)"
    );
}

#[test]
fn split_mode_loads_the_lines_a_row_needs_not_the_rows_in_between() {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(
        repo.path().join("big.txt"),
        (0..6000).map(|i| format!("old {i}\n")).collect::<String>(),
    )
    .unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "base"]);
    std::fs::write(
        repo.path().join("big.txt"),
        (0..6000).map(|i| format!("new {i}\n")).collect::<String>(),
    )
    .unwrap();
    let mut state = opened(repo.path());
    state.settle(1, Duration::from_secs(30));
    state.set_mode(crate::state::DiffMode::Split);
    let (index, _) = state.entry_at(0).unwrap();
    let doc_rows = state.body_rows(index);
    assert_eq!(doc_rows, 6001, "one header row and 6000 paired rows");

    let first_row = state.layout.offsets[0] + 1 + 3000;
    state.on_visible(first_row as usize..first_row as usize + 30);
    let requested = &state.doc(index).unwrap().requested;
    let left_chunk = (1 + 3000) / crate::state::CHUNK_LINES;
    let right_chunk = (1 + 6000 + 3000) / crate::state::CHUNK_LINES;
    assert!(
        requested.contains(&left_chunk),
        "the removed side is loaded: {requested:?}"
    );
    assert!(
        requested.contains(&right_chunk),
        "the added side is loaded: {requested:?}"
    );
    assert!(
        !requested
            .iter()
            .any(|c| *c > left_chunk + 1 && *c < right_chunk - 1),
        "the thousands of lines between the two sides are not requested: {requested:?}"
    );
}

fn expect_copy(state: &mut State) -> (String, u64, bool) {
    for _ in 0..1000 {
        state.drain();
        if let Some(copied) = state.peek_copied() {
            return copied.clone();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no copy result arrived");
}

fn anchor(
    state: &State,
    position: usize,
    row: u64,
    offset: usize,
) -> crate::state::SelectionAnchor {
    crate::state::SelectionAnchor {
        index: state.entry_at(position).unwrap().0,
        row,
        offset,
    }
}

const FILE_LINES: [&str; 9] = [
    "line 2",
    "line 3",
    "line 4",
    "line 5",
    "changed 5",
    "extra",
    "line 6",
    "line 7",
    "line 8",
];

#[test]
fn a_selection_is_copied_from_the_documents_even_when_no_line_was_ever_loaded() {
    let repo = repo_with_files(40);
    let mut state = opened(repo.path());
    assert!(state.docs.is_empty(), "nothing was requested yet");
    let first = anchor(&state, 3, 3, 2);
    let last = anchor(&state, 9, 5, 4);
    assert!(state.start_copy(first, last, None));
    let (text, lines, truncated) = expect_copy(&mut state);

    let mut expected: Vec<String> = vec!["ne 4".to_owned()];
    expected.extend(FILE_LINES[3..].iter().map(|l| (*l).to_owned()));
    for _ in 4..9 {
        expected.extend(FILE_LINES.iter().map(|l| (*l).to_owned()));
    }
    expected.extend(FILE_LINES[..4].iter().map(|l| (*l).to_owned()));
    expected.push("chan".to_owned());
    assert_eq!(text, expected.join("\n"));
    assert_eq!(lines, expected.len() as u64);
    assert!(!truncated);
}

#[test]
fn in_split_mode_only_the_selected_column_is_copied() {
    let repo = repo_with_files(4);
    let mut state = opened(repo.path());
    state.settle(4, Duration::from_secs(30));
    state.set_mode(crate::state::DiffMode::Split);
    let (first, last) = (anchor(&state, 1, 2, 0), anchor(&state, 1, 6, 6));

    assert!(state.start_copy(first, last, Some(true)));
    let (left, ..) = expect_copy(&mut state);
    assert_eq!(left, "line 3\nline 4\nline 5\nline 6");

    let mut state = opened(repo.path());
    state.settle(4, Duration::from_secs(30));
    state.set_mode(crate::state::DiffMode::Split);
    let (first, last) = (anchor(&state, 1, 2, 0), anchor(&state, 1, 6, 6));
    assert!(state.start_copy(first, last, Some(false)));
    let (right, ..) = expect_copy(&mut state);
    assert_eq!(right, "line 3\nline 4\nchanged 5\nextra\nline 6");
}

#[test]
fn a_copy_larger_than_the_limit_is_cut_and_reported() {
    let repo = repo_with_files(10);
    let mut state = opened(repo.path());
    state.copy_limit = 100;
    assert!(state.start_copy(anchor(&state, 0, 1, 0), anchor(&state, 9, 5, 9), None));
    let (text, lines, truncated) = expect_copy(&mut state);
    assert!(truncated);
    assert!(
        text.len() < 200 && (10..90).contains(&lines),
        "{} bytes, {lines} lines",
        text.len()
    );
}

#[test]
fn copy_requests_that_make_no_sense_are_refused() {
    let repo = repo_with_files(3);
    let mut state = opened(repo.path());
    let (a, b) = (anchor(&state, 0, 1, 0), anchor(&state, 2, 1, 0));
    assert!(
        !state.start_copy(b, a, None),
        "a selection never runs backwards across files"
    );
    let unknown = crate::state::SelectionAnchor {
        index: 9999,
        row: 0,
        offset: 0,
    };
    assert!(!state.start_copy(unknown, b, None));
}

fn click_at(rt: &mut Runtime<State>, x: f32, y: f32, shift: bool) {
    if shift {
        rt.physical_input(rune::PhysicalInput::Modifiers(rune::Mods {
            shift: true,
            ..Default::default()
        }));
    }
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    if shift {
        rt.physical_input(rune::PhysicalInput::Modifiers(rune::Mods::default()));
    }
    rt.frame(1.0 / 60.0);
}

fn visible_code_texts(rt: &Runtime<State>) -> Vec<(usize, u64, [f32; 4])> {
    let viewport = rect_of(rt, "viewport").unwrap();
    rt.elements()
        .into_iter()
        .filter_map(|e| {
            let key = e.key?;
            let rest = key.strip_prefix('t')?;
            let mut parts = rest.split(':');
            let (index, row) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
            let inside =
                e.rect[1] >= viewport[1] && e.rect[1] + e.rect[3] <= viewport[1] + viewport[3];
            inside.then_some((index, row, e.rect))
        })
        .collect()
}

#[test]
fn a_selection_extended_after_scrolling_far_away_copies_the_text_in_between_to_the_clipboard() {
    let repo = repo_with_files(30);
    let state = opened(repo.path());
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    let clipboard = rune::clipboard::memory();
    rt.use_clipboard(&clipboard);
    step(&mut rt);

    let first_position = 1;
    let (first_index, _) = rt.state.entry_at(first_position).unwrap();
    let anchor_key = format!("t{first_index}:3");
    let anchor_rect = loop {
        if let Some(rect) = rect_of(&rt, &anchor_key) {
            break rect;
        }
        rt.wheel(900.0, 300.0, -100.0);
        step(&mut rt);
    };
    click_at(
        &mut rt,
        anchor_rect[0] + 1.0,
        anchor_rect[1] + anchor_rect[3] / 2.0,
        false,
    );

    for _ in 0..40 {
        rt.wheel(900.0, 300.0, -400.0);
        step(&mut rt);
        if rect_of(&rt, &anchor_key).is_none()
            && rt.state.current.is_some_and(|c| c > first_position + 6)
        {
            break;
        }
    }
    assert!(
        rect_of(&rt, &anchor_key).is_none(),
        "the anchor line is no longer on screen"
    );
    let (last_index, last_row, last_rect) = visible_code_texts(&rt)
        .into_iter()
        .filter(|(_, row, _)| *row >= 1)
        .max_by(|a, b| a.2[1].partial_cmp(&b.2[1]).unwrap())
        .expect("code lines are visible");
    let last_position = rt
        .state
        .files
        .iter()
        .position(|&i| i == last_index)
        .unwrap();
    assert!(
        last_position > first_position + 4,
        "the selection crosses several files that were never shown"
    );
    click_at(
        &mut rt,
        last_rect[0] + last_rect[2] - 1.0,
        last_rect[1] + last_rect[3] / 2.0,
        true,
    );

    rt.key(rune::KeyEvent {
        key: rune::Key::Command('c'),
        mods: rune::Mods {
            cmd: true,
            ctrl: true,
            ..Default::default()
        },
    });
    for _ in 0..300 {
        step(&mut rt);
        if clipboard.text().is_some() {
            break;
        }
    }
    let copied = clipboard
        .text()
        .expect("the copy reached the clipboard")
        .to_string();

    let mut expected: Vec<String> = FILE_LINES[2..].iter().map(|l| (*l).to_owned()).collect();
    for _ in first_position + 1..last_position {
        expected.extend(FILE_LINES.iter().map(|l| (*l).to_owned()));
    }
    expected.extend(
        FILE_LINES[..last_row as usize]
            .iter()
            .map(|l| (*l).to_owned()),
    );
    assert_eq!(copied, expected.join("\n"));
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("copiées")),
        "{:?}",
        rt.state.notice
    );
}

fn pump(state: &mut State, until: impl Fn(&State) -> bool) {
    for _ in 0..2000 {
        state.drain();
        if until(state) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the condition was never met");
}

fn has_chunk(state: &State, position: usize, chunk: u64) -> bool {
    let (index, _) = state.entry_at(position).unwrap();
    state
        .doc(index)
        .is_some_and(|d| d.chunks.contains_key(&chunk))
}

#[test]
fn documents_far_from_the_viewport_are_released_and_load_again_when_visited() {
    let repo = repo_with_files(160);
    let mut state = opened(repo.path());
    state.max_loaded_lines = 100;
    let total = state.files.len();

    for position in (0..total).step_by(8) {
        let row = state.layout.offsets[position] as usize;
        state.on_visible(row..row + 30);
        pump(&mut state, |s| has_chunk(s, position, 0));
    }
    assert!(
        !has_chunk(&state, 0, 0) && !has_chunk(&state, 20, 0),
        "files far behind the viewport were released"
    );
    let loaded = state.loaded_lines();
    assert!(loaded < 120 * 10, "memory stays bounded: {loaded} lines");
    let last = total - 1;
    assert!(
        has_chunk(&state, last, 0) || has_chunk(&state, total - 8, 0),
        "files near the viewport stay"
    );

    state.on_visible(0..30);
    pump(&mut state, |s| has_chunk(s, 0, 0));
    assert!(
        has_chunk(&state, 0, 0),
        "scrolling back loads the file again"
    );
    assert!(
        !has_chunk(&state, total - 1, 0),
        "now the far end is released"
    );
}

#[test]
fn a_huge_document_keeps_only_the_chunks_around_the_viewport() {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(
        repo.path().join("big.txt"),
        (0..9000).map(|i| format!("row {i}\n")).collect::<String>(),
    )
    .unwrap();
    let mut state = opened(repo.path());
    state.max_loaded_lines = 1000;
    state.settle(1, Duration::from_secs(30));
    let base = state.layout.offsets[0] as usize + 1;

    for line in (0..9000).step_by(200) {
        state.on_visible(base + line..base + line + 40);
        let chunk = (line as u64 + 1) / crate::state::CHUNK_LINES;
        pump(&mut state, |s| has_chunk(s, 0, chunk));
    }
    assert!(
        !has_chunk(&state, 0, 0),
        "the start of the document was released"
    );
    let last_chunk = (8800 + 1) / crate::state::CHUNK_LINES;
    assert!(
        has_chunk(&state, 0, last_chunk),
        "the chunk under the viewport is loaded"
    );
    let loaded = state.loaded_lines();
    assert!(
        loaded <= (2 * 6 + 4) * crate::state::CHUNK_LINES as usize,
        "bounded around the viewport: {loaded}"
    );
}

fn repo_with_long_line() -> (TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    let long = |tag: &str| {
        format!(
            "let value = {tag}({});\n",
            (0..80)
                .map(|i| format!("argument_{i}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    std::fs::write(
        dir.path().join("a.rs"),
        format!("fn short() {{}}\n{}fn tail() {{}}\n", long("compute")),
    )
    .unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    std::fs::write(
        dir.path().join("a.rs"),
        format!("fn short() {{}}\n{}fn tail() {{}}\n", long("changed")),
    )
    .unwrap();
    (dir, long("changed").trim_end().to_owned())
}

#[test]
fn long_lines_are_not_truncated_and_the_diff_scrolls_sideways_with_sticky_line_numbers() {
    let (repo, long_line) = repo_with_long_line();
    let mut state = opened(repo.path());
    state.settle(1, Duration::from_secs(30));
    let (index, _) = state.entry_at(0).unwrap();
    let mut rt = runtime(state);
    step(&mut rt);

    let key = format!("t{index}:3");
    let viewport = rect_of(&rt, "viewport").unwrap();
    let before = rect_of(&rt, &key).expect("the added long line is on screen");
    let columns = long_line.chars().count() as f32;
    assert!(
        before[2] > columns * 5.0,
        "the text keeps its natural width: {} for {columns} columns",
        before[2]
    );
    assert!(before[2] > viewport[2], "it is wider than the viewport");
    assert!(
        rt.elements().iter().all(|e| e.stuck == 0.0),
        "nothing is stuck before scrolling"
    );

    rt.wheel_xy(viewport[0] + 300.0, viewport[1] + 120.0, -600.0, 0.0);
    for _ in 0..30 {
        step(&mut rt);
    }
    let after = rect_of(&rt, &key).unwrap();
    assert!(
        before[0] - after[0] > 300.0,
        "the code moved left by the scroll: {} -> {}",
        before[0],
        after[0]
    );
    let stuck: Vec<f32> = rt
        .elements()
        .iter()
        .map(|e| e.stuck)
        .filter(|s| *s > 0.0)
        .collect();
    assert!(
        stuck.len() >= 3,
        "the line number columns stay at the left edge: {stuck:?}"
    );
}

#[test]
fn in_split_mode_long_lines_wrap_inside_their_column_and_nothing_scrolls_sideways() {
    let (repo, _) = repo_with_long_line();
    let mut state = opened(repo.path());
    state.settle(1, Duration::from_secs(30));
    let (index, _) = state.entry_at(0).unwrap();
    state.set_mode(crate::state::DiffMode::Split);
    let mut rt = runtime(state);
    step(&mut rt);

    let viewport = rect_of(&rt, "viewport").unwrap();
    let left = rect_of(&rt, &format!("t{index}:2:L")).expect("the removed long line is on screen");
    let right = rect_of(&rt, &format!("t{index}:2:R")).expect("the added long line is on screen");
    for cell in [left, right] {
        assert!(
            cell[3] > 4.0 * 25.0,
            "the line wrapped onto several lines: height {}",
            cell[3]
        );
        assert!(
            cell[0] + cell[2] <= viewport[0] + viewport[2] + 1.0,
            "it stays inside the viewport"
        );
    }
    rt.wheel_xy(viewport[0] + 300.0, viewport[1] + 120.0, 600.0, 0.0);
    for _ in 0..10 {
        step(&mut rt);
    }
    let again = rect_of(&rt, &format!("t{index}:2:L")).unwrap();
    assert!(
        (again[0] - left[0]).abs() < 1.0,
        "split mode has no horizontal scroll"
    );
}

#[test]
fn left_and_right_arrows_scroll_a_wide_diff_sideways_from_the_keyboard() {
    let (repo, _) = repo_with_long_line();
    let mut state = opened(repo.path());
    state.settle(1, Duration::from_secs(30));
    let (index, _) = state.entry_at(0).unwrap();
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    step(&mut rt);
    let key = format!("t{index}:3");
    let start = rect_of(&rt, &key).unwrap()[0];

    press(&mut rt, rune::Key::Right);
    press(&mut rt, rune::Key::Right);
    let moved = rect_of(&rt, &key).unwrap()[0];
    assert!(
        (start - moved - 192.0).abs() < 2.0,
        "two presses move the code by two steps: {start} -> {moved}"
    );
    press(&mut rt, rune::Key::Left);
    let back = rect_of(&rt, &key).unwrap()[0];
    assert!((start - back - 96.0).abs() < 2.0, "one step back: {back}");
    for _ in 0..4 {
        press(&mut rt, rune::Key::Left);
    }
    assert!(
        (rect_of(&rt, &key).unwrap()[0] - start).abs() < 2.0,
        "it stops at the left edge"
    );
}

fn sized_runtime(files: usize, width: f32, height: f32) -> (TempDir, Runtime<State>) {
    let repo = repo_with_files(files);
    let state = opened(repo.path());
    let mut rt = Runtime::new(
        state,
        view,
        (width, height),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
        std::thread::sleep(Duration::from_millis(5));
        rt.state.drain();
    }
    (repo, rt)
}

#[test]
fn the_file_list_never_takes_more_than_its_share_of_a_small_window() {
    let (_repo, small) = sized_runtime(6, 720.0, 560.0);
    let divider = rect_of(&small, "divider").unwrap();
    assert!(
        divider[0] <= 720.0 * 0.45 + 1.0,
        "the list is {} wide in a 720 window",
        divider[0]
    );
    let viewport = rect_of(&small, "viewport").unwrap();
    assert!(
        viewport[2] >= 380.0,
        "the diff keeps room to read: {}",
        viewport[2]
    );

    let (_repo, large) = sized_runtime(6, 1440.0, 900.0);
    let divider = rect_of(&large, "divider").unwrap();
    assert!(
        (divider[0] - crate::state::LIST_WIDTH_DEFAULT).abs() < 1.0,
        "the chosen width is kept when there is room"
    );
}

#[test]
fn every_control_stays_inside_the_window_at_the_smallest_supported_size() {
    let (_repo, rt) = sized_runtime(6, 720.0, 480.0);
    for key in [
        "open",
        "history",
        "ignored",
        "branch-name",
        "profiles",
        "refresh",
        "theme",
        "commit-message",
        "commit",
        "Non indexés",
        "Indexés",
        "filter",
        "mode-unified",
        "mode-split",
        "viewport",
        "divider",
    ] {
        let rect = rect_of(&rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
        assert!(
            rect[0] >= 0.0 && rect[0] + rect[2] <= 720.5,
            "{key} leaves the window horizontally: {rect:?}"
        );
        assert!(
            rect[1] >= 0.0 && rect[1] + rect[3] <= 480.5,
            "{key} leaves the window vertically: {rect:?}"
        );
    }
}

fn scroll_to_file(rt: &mut Runtime<State>, position: usize) {
    rt.update(move |s, tx| {
        s.jump_to(position);
        let index = s.files[position];
        crate::keys::align_file(tx, index);
    });
    for _ in 0..4 {
        step(rt);
    }
}

fn reading_path(rt: &Runtime<State>) -> Option<std::path::PathBuf> {
    rt.state.capture_anchor().map(|a| a.path)
}

fn refresh_and_settle(rt: &mut Runtime<State>) {
    let before = rt.state.repo.as_ref().unwrap().generation;
    rt.update(|s, _| s.refresh());
    for _ in 0..200 {
        step(rt);
        if rt.state.repo.as_ref().unwrap().generation != before {
            break;
        }
    }
    for _ in 0..6 {
        step(rt);
    }
}

#[test]
fn a_manual_refresh_shows_an_external_change_and_keeps_the_file_being_read() {
    let (repo, mut rt) = keyboard_runtime(30);
    scroll_to_file(&mut rt, 12);
    for _ in 0..4 {
        step(&mut rt);
    }
    let reading = reading_path(&rt).unwrap();
    let total = rt.state.count(ViewKind::Unstaged);

    std::fs::write(repo.path().join("brand_new.txt"), "x\n").unwrap();
    refresh_and_settle(&mut rt);
    for _ in 0..4 {
        step(&mut rt);
    }

    assert_eq!(rt.state.count(ViewKind::Unstaged), total + 1);
    assert_eq!(reading_path(&rt), Some(reading));
}

#[test]
fn when_the_read_file_disappears_reading_resumes_on_the_next_one_with_a_notice() {
    let (repo, mut rt) = keyboard_runtime(30);
    scroll_to_file(&mut rt, 12);
    let reading = reading_path(&rt).unwrap();
    let paths: Vec<_> = rt
        .state
        .files
        .iter()
        .map(|&i| rt.state.entries()[i].path.clone())
        .collect();
    let at = paths.iter().position(|p| *p == reading).unwrap();
    let next = paths[at + 1].clone();

    git(repo.path(), &["checkout", "--", reading.to_str().unwrap()]);
    refresh_and_settle(&mut rt);

    assert!(
        !rt.state
            .files
            .iter()
            .any(|&i| rt.state.entries()[i].path == reading)
    );
    assert_eq!(reading_path(&rt), Some(next));
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("lecture reprise")),
        "{:?}",
        rt.state.notice
    );
}

#[test]
fn the_reading_position_of_each_view_is_remembered_across_switches() {
    let (_repo, mut rt) = keyboard_runtime(30);
    scroll_to_file(&mut rt, 15);
    let unstaged = reading_path(&rt).unwrap();
    press(&mut rt, rune::Key::Text("2".to_owned()));
    for _ in 0..4 {
        step(&mut rt);
    }
    assert_eq!(rt.state.view, ViewKind::Staged);
    press(&mut rt, rune::Key::Text("1".to_owned()));
    for _ in 0..8 {
        step(&mut rt);
    }
    assert_eq!(reading_path(&rt), Some(unstaged));
}

#[test]
fn a_refresh_that_is_not_newer_than_what_is_shown_is_ignored() {
    let repo = repo_with_files(5);
    let mut state = opened(repo.path());
    let shown = state.repo.as_ref().unwrap().snapshot.clone();
    let mut stale = (*shown).clone();
    stale.generation = state.repo.as_ref().unwrap().generation;
    state.notice = None;
    assert!(!state.apply_refresh(1, stale, false));
    assert!(state.notice.is_none());
}

#[test]
fn the_watcher_reports_an_external_edit_without_any_manual_action() {
    let repo = repo_with_files(5);
    let mut state = opened(repo.path());
    pump(&mut state, |s| s.watch == crate::state::WatchStatus::Active);
    let total = state.count(ViewKind::Unstaged);
    std::fs::write(repo.path().join("watched.txt"), "x\n").unwrap();
    pump(&mut state, |s| s.count(ViewKind::Unstaged) == total + 1);
}

#[test]
fn shutdown_stops_the_engine_and_is_safe_to_repeat() {
    let repo = repo_with_files(3);
    let mut state = opened(repo.path());
    pump(&mut state, |s| s.watch == crate::state::WatchStatus::Active);
    let started = std::time::Instant::now();
    state.shutdown();
    assert!(started.elapsed() < Duration::from_secs(5));
    state.shutdown();
    state.refresh();
}

#[test]
fn returning_to_the_window_refreshes_once_and_not_in_a_burst() {
    let repo = repo_with_files(3);
    let mut state = opened(repo.path());
    let before = state.refresh_request;
    state.refresh_on_focus();
    state.refresh_on_focus();
    assert_eq!(state.refresh_request, before + 1);
}

#[test]
fn the_wordmark_fits_its_column_at_the_smallest_supported_size() {
    let (_repo, rt) = sized_runtime(6, 720.0, 480.0);
    let right = |needle: &str| {
        rt.elements()
            .into_iter()
            .find(|e| e.text.as_deref() == Some(needle))
            .map(|e| e.rect[0] + e.rect[2])
            .unwrap()
    };
    let context_left = rect_of(&rt, "open").unwrap()[0];
    assert!(
        right("poulpe") <= context_left,
        "{} > {context_left}",
        right("poulpe")
    );
}
