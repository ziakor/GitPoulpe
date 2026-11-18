use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use rune::{Runtime, TextEdit};

use super::{click_at, press, rect_of, runtime, step};
use crate::state::{Screen, State};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Ada")
        .env("GIT_AUTHOR_EMAIL", "ada@example.org")
        .env("GIT_COMMITTER_NAME", "Ada")
        .env("GIT_COMMITTER_EMAIL", "ada@example.org")
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn settle(rt: &mut Runtime<State>, until: impl Fn(&State) -> bool) {
    for _ in 0..120 {
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

struct Env {
    dir: tempfile::TempDir,
    source: tempfile::TempDir,
    parent: tempfile::TempDir,
}

fn welcome() -> (Env, Runtime<State>) {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fakessh");
    std::fs::write(
        &script,
        include_str!("../../../gitpoulpe-git/tests/common/fake_ssh.sh"),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.path().join("mode"), "ok").unwrap();
    let source = tempfile::tempdir().unwrap();
    git(source.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(source.path().join("a.txt"), "one\n").unwrap();
    git(source.path(), &["add", "-A"]);
    git(source.path(), &["commit", "-q", "-m", "first"]);
    let engine_git = gitpoulpe_git::Git::new()
        .with_env("GIT_SSH_COMMAND", script.as_os_str())
        .with_env("FAKESSH_DIR", dir.path().as_os_str());
    let mut state = State::new(None, rune::prefs::Prefs::memory());
    state.engine_git = Some(engine_git);
    state.setup(Arc::new(|| {}));
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    (
        Env {
            dir,
            source,
            parent: tempfile::tempdir().unwrap(),
        },
        rt,
    )
}

fn fill(rt: &mut Runtime<State>, url: &str, parent: &Path, name: &str) {
    click(rt, "clone");
    settle(rt, |s| s.clone.open);
    rt.state.clone.url = TextEdit::new(url);
    rt.state.set_clone_parent(parent.to_path_buf());
    rt.state.clone.name = TextEdit::new(name);
    step(rt);
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_clone_opens_the_repository_on_the_fetched_branch() {
    let (env, mut rt) = welcome();
    let url = format!("file://{}", env.source.path().display());
    fill(&mut rt, &url, env.parent.path(), "copy");
    click(&mut rt, "clone-start");
    settle(&mut rt, |s| s.screen == Screen::Ready && s.repo.is_some());
    assert!(!rt.state.clone.open);
    let repo = rt.state.repo.as_ref().unwrap();
    assert_eq!(repo.name, "copy");
    assert_eq!(repo.snapshot.branch.head.as_deref(), Some("main"));
    assert!(env.parent.path().join("copy/a.txt").exists());
}

#[test]
fn the_folder_name_is_proposed_from_the_url_when_left_empty() {
    let (env, mut rt) = welcome();
    let url = format!("file://{}", env.source.path().display());
    fill(&mut rt, &url, env.parent.path(), "");
    let proposed = rt.state.clone.proposed_name();
    assert!(!proposed.is_empty());
    click(&mut rt, "clone-start");
    settle(&mut rt, |s| s.screen == Screen::Ready);
    assert!(env.parent.path().join(&proposed).join("a.txt").exists());
}

#[test]
fn a_refused_clone_explains_itself_keeps_the_panel_and_creates_nothing() {
    let (env, mut rt) = welcome();
    fill(
        &mut rt,
        "https://user:tok@example.org/r.git",
        env.parent.path(),
        "copy",
    );
    click(&mut rt, "clone-start");
    settle(&mut rt, |s| s.clone.error.is_some() && !s.clone.busy);
    assert!(shows(&rt, "contient des identifiants"));
    assert!(rt.state.clone.open);
    assert!(listing(env.parent.path()).is_empty());
    assert!(shows(&rt, "Réessayer"));
}

#[test]
fn a_cancelled_clone_leaves_no_folder_and_can_be_retried() {
    let (env, mut rt) = welcome();
    let url = format!("ssh://git@fakehost{}", env.source.path().display());
    std::fs::write(env.dir.path().join("mode"), "hang").unwrap();
    fill(&mut rt, &url, env.parent.path(), "copy");
    click(&mut rt, "clone-start");
    for _ in 0..40 {
        step(&mut rt);
        if env.dir.path().join("sleeper").exists() {
            break;
        }
    }
    assert!(shows(&rt, "Clonage ·"));
    click(&mut rt, "clone-stop");
    settle(&mut rt, |s| s.clone.error.is_some() && !s.clone.busy);
    assert!(shows(&rt, "opération annulée"));
    assert!(listing(env.parent.path()).is_empty());
    std::fs::write(env.dir.path().join("mode"), "ok").unwrap();
    click(&mut rt, "clone-start");
    settle(&mut rt, |s| s.screen == Screen::Ready && s.repo.is_some());
    assert_eq!(listing(env.parent.path()), ["copy"]);
}

#[test]
fn escape_closes_the_clone_panel() {
    let (_env, mut rt) = welcome();
    click(&mut rt, "clone");
    settle(&mut rt, |s| s.clone.open);
    press(&mut rt, rune::Key::Escape);
    press(&mut rt, rune::Key::Escape);
    assert!(!rt.state.clone.open);
}

#[test]
fn the_entry_buttons_fit_inside_the_card_of_the_failure_screen() {
    let (env, mut rt) = welcome();
    rt.state.open_path(env.parent.path().to_path_buf());
    settle(&mut rt, |s| matches!(s.screen, Screen::Failed { .. }));
    let rects: Vec<[f32; 4]> = ["open", "clone", "init-here", "init"]
        .iter()
        .map(|k| rect_of(&rt, k).unwrap_or_else(|| panic!("{k} missing")))
        .collect();
    let left = rects.iter().map(|r| r[0]).fold(f32::MAX, f32::min);
    let right = rects.iter().map(|r| r[0] + r[2]).fold(0.0, f32::max);
    assert!(right - left <= 520.0, "buttons span {}", right - left);
}
