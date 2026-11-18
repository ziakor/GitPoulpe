use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use rune::{Key, KeyEvent, Mods, Runtime};

use super::{click_at, focus, keyboard_runtime, press, rect_of, step};
use crate::state::{State, ViewKind};

pub(super) fn git_text(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn use_profile(state: &mut State) {
    state.new_profile();
    state.form.label.text = "Test".to_owned();
    state.form.name.text = "Ada Lovelace".to_owned();
    state.form.email.text = "ada@example.org".to_owned();
    state.save_profile();
    let id = state.form.editing.clone().unwrap();
    state.use_profile_here(Some(id));
}

pub(super) fn settle_commit(rt: &mut Runtime<State>) {
    for _ in 0..200 {
        step(rt);
        if !rt.state.writing {
            break;
        }
    }
    for _ in 0..6 {
        step(rt);
    }
    assert!(!rt.state.writing, "the commit never finished");
}

pub(super) fn staged_runtime(files: usize) -> (tempfile::TempDir, Runtime<State>) {
    let (repo, mut rt) = keyboard_runtime(files);
    use_profile(&mut rt.state);
    for _ in 0..30 {
        step(&mut rt);
        if rt.state.identity_shown.is_some() {
            break;
        }
    }
    (repo, rt)
}

fn type_chars(rt: &mut Runtime<State>, text: &str) {
    for c in text.chars() {
        press(rt, Key::Text(c.to_string()));
    }
}

pub(super) fn commits(dir: &Path) -> usize {
    git_text(dir, &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap()
}

#[test]
fn the_button_creates_a_commit_with_the_profile_identity_and_clears_the_message() {
    let (repo, mut rt) = staged_runtime(3);
    let dir = repo.path();
    let before = commits(dir);
    let head_before = git_text(dir, &["rev-parse", "HEAD"]);
    assert_eq!(
        rt.state.staged_for_commit().len(),
        1,
        "staged.txt is staged"
    );
    rt.state.commit_message.text = "Update staged.txt\n\nWith a body.".to_owned();
    step(&mut rt);

    let button = rect_of(&rt, "commit").expect("the commit button is on screen");
    click_at(
        &mut rt,
        button[0] + 10.0,
        button[1] + button[3] / 2.0,
        false,
    );
    settle_commit(&mut rt);

    assert_eq!(commits(dir), before + 1);
    assert_eq!(
        git_text(dir, &["log", "-1", "--format=%s"]),
        "Update staged.txt"
    );
    assert_eq!(
        git_text(dir, &["log", "-1", "--format=%an <%ae>"]),
        "Ada Lovelace <ada@example.org>"
    );
    assert!(rt.state.commit_message.text.is_empty());
    assert!(
        rt.state
            .notice
            .clone()
            .unwrap_or_default()
            .contains("Update staged.txt")
    );
    assert_eq!(
        rt.state.count(ViewKind::Staged),
        0,
        "nothing is staged any more"
    );
    let journal = rt.state.journal();
    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0].action, "commit");
    assert_eq!(journal[0].before.as_deref(), Some(head_before.as_str()));
    assert_eq!(
        journal[0].after.as_deref(),
        Some(git_text(dir, &["rev-parse", "HEAD"]).as_str()),
        "the journal keeps the recovery point"
    );
}

#[test]
fn control_enter_in_the_message_field_commits() {
    let (repo, mut rt) = staged_runtime(3);
    let before = commits(repo.path());
    focus(&mut rt, "commit-message");
    rt.state.commit_message.text = "From the keyboard".to_owned();
    step(&mut rt);
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    });
    settle_commit(&mut rt);
    assert_eq!(commits(repo.path()), before + 1);
    assert_eq!(
        git_text(repo.path(), &["log", "-1", "--format=%s"]),
        "From the keyboard"
    );
}

#[test]
fn an_empty_message_or_a_stale_index_commits_nothing_and_keeps_what_was_typed() {
    let (repo, mut rt) = staged_runtime(3);
    let dir = repo.path();
    let before = commits(dir);

    rt.state.commit_message.text = "   ".to_owned();
    rt.state.commit();
    settle_commit(&mut rt);
    assert_eq!(
        rt.state.notice.as_deref(),
        Some("Écris un message de commit.")
    );

    rt.state.commit_message.text = "Will not be created".to_owned();
    step(&mut rt);
    let other = rt
        .state
        .entries()
        .iter()
        .find(|e| !e.is_staged())
        .unwrap()
        .path
        .clone();
    let output = Command::new("git")
        .current_dir(dir)
        .args(["add", "--"])
        .arg(&other)
        .output()
        .unwrap();
    assert!(output.status.success());
    rt.state.commit();
    settle_commit(&mut rt);

    assert_eq!(commits(dir), before, "no commit");
    let notice = rt.state.notice.clone().unwrap_or_default();
    assert!(notice.contains("a changé depuis son affichage"), "{notice}");
    assert_eq!(
        rt.state.commit_message.text, "Will not be created",
        "the message is kept"
    );
}

#[test]
fn a_refusing_hook_shows_its_output_keeps_the_message_and_allows_a_retry() {
    let (repo, mut rt) = staged_runtime(3);
    let dir = repo.path();
    let before = commits(dir);
    let hook = dir.join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(
        &hook,
        "#!/bin/sh\necho 'style check failed on staged.txt' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    rt.state.commit_message.text = "Needs a retry".to_owned();
    step(&mut rt);

    rt.state.commit();
    settle_commit(&mut rt);
    assert_eq!(commits(dir), before);
    let report = rt.state.commit_report.clone().unwrap_or_default();
    assert!(
        report.contains("style check failed on staged.txt"),
        "{report}"
    );
    assert!(report.contains("hook"), "{report}");
    assert_eq!(rt.state.commit_message.text, "Needs a retry");

    std::fs::remove_file(&hook).unwrap();
    rt.state.commit();
    settle_commit(&mut rt);
    assert_eq!(commits(dir), before + 1, "the retry succeeds");
    assert!(rt.state.commit_report.is_none());
}

#[test]
fn typing_in_the_message_field_never_triggers_a_shortcut() {
    let (repo, mut rt) = staged_runtime(3);
    let unstaged_before = rt.state.count(ViewKind::Unstaged);
    focus(&mut rt, "commit-message");
    type_chars(&mut rt, "sux jk m r 12 /");
    assert!(!rt.state.writing, "no stage or unstage was started");
    assert_eq!(rt.state.count(ViewKind::Unstaged), unstaged_before);
    assert_eq!(
        rt.state.view,
        ViewKind::Unstaged,
        "the digits did not switch the view"
    );
    assert!(
        rt.state.commit_message.text.contains("sux"),
        "{:?}",
        rt.state.commit_message.text
    );
    drop(repo);
}

#[test]
fn shortcuts_do_nothing_behind_the_profile_screen() {
    let (repo, mut rt) = staged_runtime(3);
    let before = rt.state.count(ViewKind::Staged);
    rt.state.open_profiles();
    for _ in 0..4 {
        step(&mut rt);
    }
    rt.state.jump_to(0);
    for letter in ["s", "x", "S", "m", "r", "2"] {
        press(&mut rt, Key::Text(letter.to_owned()));
    }
    assert!(
        !rt.state.writing,
        "no stage was started behind the profile screen"
    );
    assert_eq!(rt.state.count(ViewKind::Staged), before);
    assert_eq!(
        rt.state.view,
        ViewKind::Unstaged,
        "the digit did not switch the view"
    );
    assert!(rt.state.marked.is_empty(), "x did not mark a file");

    focus(&mut rt, "pf-name");
    type_chars(&mut rt, "sux");
    assert!(
        rt.state.form.name.text.ends_with("sux"),
        "{:?}",
        rt.state.form.name.text
    );
    drop(repo);
}
