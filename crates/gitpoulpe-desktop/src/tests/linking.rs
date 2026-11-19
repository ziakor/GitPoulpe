use std::path::Path;
use std::process::Command;

use gitpoulpe_core::Favorite;
use rune::{Runtime, TextEdit};

use super::{click_at, rect_of, runtime, step};
use crate::state::{Screen, State, ViewKind};

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
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn settle(rt: &mut Runtime<State>, until: impl Fn(&State) -> bool) {
    for _ in 0..300 {
        step(rt);
        if until(&rt.state) {
            for _ in 0..8 {
                step(rt);
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the screen never settled");
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn repo(remote: Option<&str>) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    if let Some(url) = remote {
        git(p, &["remote", "add", "origin", url]);
    }
    let sha = git(p, &["rev-parse", "HEAD"]);
    (dir, sha)
}

fn open_link(rt: &mut Runtime<State>, link: &str) {
    rt.state.open_deep_link(None);
    rt.state.links.input = TextEdit::new(link);
    step(rt);
    click(rt, "lk-analyse");
    settle(rt, |s| !s.links.busy);
}

#[test]
fn a_link_to_a_commit_joins_the_matching_local_repository() {
    let (dir, sha) = repo(Some("git@github.com:Owner/Repo.git"));
    let mut rt = runtime(super::opened(dir.path()));
    open_link(
        &mut rt,
        &format!("https://github.com/owner/repo/commit/{}", &sha[..12]),
    );
    assert!(shows(&rt, "Lien reconnu"));
    assert_eq!(rt.state.links.matches.as_ref().map(Vec::len), Some(1));
    click(&mut rt, "lk-open0");
    settle(&mut rt, |s| s.view == ViewKind::Commit);
    assert!(!rt.state.links.open);
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "");
}

#[test]
fn a_link_to_another_known_repository_opens_it_only_after_the_explicit_click() {
    let (current, _) = repo(None);
    let (other, _) = repo(Some("https://gitlab.example.com/group/sub/project.git"));
    let mut rt = runtime(super::opened(current.path()));
    rt.state
        .favorites
        .book
        .add(Favorite::Local {
            path: other.path().display().to_string(),
            name: "other".into(),
        })
        .unwrap();
    open_link(
        &mut rt,
        "gitpoulpe://open/gitlab.example.com/group/sub/project",
    );
    assert_eq!(rt.state.links.matches.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        rt.state.repo.as_ref().unwrap().path.canonicalize().unwrap(),
        current.path().canonicalize().unwrap()
    );
    click(&mut rt, "lk-open0");
    settle(&mut rt, |s| {
        s.repo
            .as_ref()
            .is_some_and(|r| r.path.canonicalize().ok() == other.path().canonicalize().ok())
            && matches!(s.screen, Screen::Ready)
    });
}

#[test]
fn an_unknown_repository_and_a_missing_commit_give_an_explicit_next_step() {
    let (dir, _) = repo(Some("https://github.com/owner/repo.git"));
    let mut rt = runtime(super::opened(dir.path()));
    open_link(&mut rt, "https://github.com/someone/else/pull/3");
    assert!(shows(&rt, "Aucun dépôt local connu"));
    assert!(rt.state.links.matches.as_ref().unwrap().is_empty());
    open_link(
        &mut rt,
        "https://github.com/owner/repo/commit/0123456789abcdef",
    );
    click(&mut rt, "lk-open0");
    step(&mut rt);
    assert!(rt.state.links.open);
    assert!(shows(&rt, "n'est pas dans ce dépôt local"));
    assert_ne!(rt.state.view, ViewKind::Commit);
}

#[test]
fn a_pull_link_opens_the_pull_screen_without_choosing_an_account() {
    let (dir, _) = repo(Some("https://github.com/owner/repo.git"));
    let mut rt = runtime(super::opened(dir.path()));
    open_link(&mut rt, "https://github.com/owner/repo/pull/12");
    click(&mut rt, "lk-open0");
    settle(&mut rt, |s| s.pulls.open);
    assert!(shows(&rt, "Aucun compte connecté"));
}

#[test]
fn malformed_and_hostile_links_never_reach_git() {
    let (dir, _) = repo(Some("https://github.com/owner/repo.git"));
    let mut rt = runtime(super::opened(dir.path()));
    for link in [
        "",
        "javascript:alert(1)",
        "https://github.com/owner/repo?action=merge",
        "https://github.com/owner/repo/pull/1;rm -rf ~",
        "gitpoulpe://open/github.com/owner/repo/../../etc",
        "https://github.com/owner/repo/--upload-pack=x",
    ] {
        rt.state.open_deep_link(None);
        rt.state.links.input = TextEdit::new(link);
        step(&mut rt);
        click(&mut rt, "lk-analyse");
        step(&mut rt);
        assert!(rt.state.links.error.is_some(), "{link}");
        assert!(
            rt.state.links.link.is_none() && !rt.state.links.busy,
            "{link}"
        );
        assert!(rect_of(&rt, "lk-open0").is_none(), "{link}");
    }
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "");
}

#[test]
fn only_the_gitpoulpe_scheme_is_taken_from_the_command_line() {
    let args = |v: &[&str]| v.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
    assert_eq!(
        crate::deep_link::link_from_args(args(&[
            "gitpoulpe",
            "--x",
            "gitpoulpe://open/github.com/o/r"
        ])),
        Some("gitpoulpe://open/github.com/o/r".to_owned())
    );
    assert_eq!(
        crate::deep_link::link_from_args(args(&["gitpoulpe", "https://github.com/o/r", "/tmp"])),
        None
    );
}
