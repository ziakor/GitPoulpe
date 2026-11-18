use std::path::Path;
use std::process::Command;

use rune::{Runtime, TextEdit};

use super::{click_at, press, rect_of, runtime, step};
use crate::state::State;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.org")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.org")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
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

fn idle(s: &State) -> bool {
    !s.remotes.busy && !s.remotes.loading
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "first commit"]);
    git(
        p,
        &[
            "remote",
            "add",
            "origin",
            "https://user:s3cr3t@example.org/a.git",
        ],
    );
    git(p, &["remote", "add", "mirror", "git@example.org:b.git"]);
    let head = git(p, &["rev-parse", "HEAD"]);
    git(p, &["update-ref", "refs/remotes/mirror/main", &head]);
    git(p, &["config", "branch.main.remote", "mirror"]);
    git(p, &["config", "branch.main.merge", "refs/heads/main"]);
    dir
}

fn screen(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    click(&mut rt, "branches-remotes");
    settle(&mut rt, |s| s.remotes.open && idle(s));
    rt
}

fn pick(rt: &mut Runtime<State>, name: &str) {
    rt.state.select_remote(name);
    for _ in 0..6 {
        step(rt);
    }
}

fn type_into(rt: &mut Runtime<State>, set: impl FnOnce(&mut State)) {
    set(&mut rt.state);
    step(rt);
}

fn remotes(dir: &Path) -> Vec<String> {
    git(dir, &["remote"]).lines().map(str::to_owned).collect()
}

#[test]
fn the_remote_screen_lists_remotes_and_never_shows_credentials() {
    let dir = repo();
    let mut rt = screen(dir.path());
    let listed: Vec<String> = rt
        .state
        .remotes
        .remotes
        .iter()
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(listed, remotes(dir.path()));
    assert!(shows(&rt, "https://***@example.org/a.git"));
    pick(&mut rt, "origin");
    assert!(shows(&rt, "identifiants"));
    for element in rt.elements() {
        if let Some(text) = element.text.as_deref() {
            assert!(!text.contains("s3cr3t"), "credential leaked: {text}");
        }
    }
}

#[test]
fn a_remote_is_added_with_its_url_and_an_invalid_one_changes_nothing() {
    let dir = repo();
    let mut rt = screen(dir.path());
    let before = git(dir.path(), &["config", "--list", "--local"]);
    type_into(&mut rt, |s| {
        s.remotes.new_name = TextEdit::new("extra");
        s.remotes.new_url = TextEdit::new("ext::sh -c id");
    });
    click(&mut rt, "rm-add");
    settle(&mut rt, idle);
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("URL refusée"))
    );
    assert_eq!(git(dir.path(), &["config", "--list", "--local"]), before);
    type_into(&mut rt, |s| {
        s.remotes.new_url = TextEdit::new("https://example.org/extra.git");
    });
    click(&mut rt, "rm-add");
    settle(&mut rt, |s| idle(s) && s.remotes.remotes.len() == 3);
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "extra"]),
        "https://example.org/extra.git"
    );
}

#[test]
fn a_url_changed_behind_the_screen_is_not_overwritten() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "mirror");
    git(
        dir.path(),
        &["remote", "set-url", "mirror", "git@example.org:moved.git"],
    );
    type_into(&mut rt, |s| {
        s.remotes.url = TextEdit::new("git@example.org:mine.git")
    });
    click(&mut rt, "rm-url-save");
    settle(&mut rt, idle);
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("a changé"))
    );
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "mirror"]),
        "git@example.org:moved.git"
    );
    pick(&mut rt, "mirror");
    type_into(&mut rt, |s| {
        s.remotes.url = TextEdit::new("git@example.org:mine.git")
    });
    click(&mut rt, "rm-url-save");
    settle(&mut rt, idle);
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "mirror"]),
        "git@example.org:mine.git"
    );
}

#[test]
fn the_push_url_is_set_then_cleared() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "mirror");
    type_into(&mut rt, |s| {
        s.remotes.push_url = TextEdit::new("ssh://example.org/push.git")
    });
    click(&mut rt, "rm-push-save");
    settle(&mut rt, idle);
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "--push", "mirror"]),
        "ssh://example.org/push.git"
    );
    assert!(shows(&rt, "envoi différent"));
    pick(&mut rt, "mirror");
    click(&mut rt, "rm-push-clear");
    settle(&mut rt, idle);
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "--push", "mirror"]),
        "git@example.org:b.git"
    );
}

#[test]
fn renaming_keeps_the_tracking_branch_and_a_taken_name_is_refused() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "mirror");
    type_into(&mut rt, |s| s.remotes.rename = TextEdit::new("origin"));
    click(&mut rt, "rm-rename-save");
    settle(&mut rt, idle);
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("déjà ce nom"))
    );
    assert_eq!(remotes(dir.path()), ["mirror", "origin"]);
    pick(&mut rt, "mirror");
    type_into(&mut rt, |s| s.remotes.rename = TextEdit::new("backup"));
    click(&mut rt, "rm-rename-save");
    settle(&mut rt, |s| {
        idle(s) && s.remotes.remotes.iter().any(|r| r.name == "backup")
    });
    assert_eq!(git(dir.path(), &["config", "branch.main.remote"]), "backup");
    assert_eq!(
        git(dir.path(), &["rev-parse", "refs/remotes/backup/main"]),
        git(dir.path(), &["rev-parse", "HEAD"])
    );
}

#[test]
fn removal_states_its_impact_gives_the_restore_command_and_escape_cancels_first() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "mirror");
    click(&mut rt, "rm-remove");
    settle(&mut rt, |s| {
        matches!(&s.remotes.confirm_remove, Some((_, Some(_))))
    });
    assert!(shows(&rt, "perdra son suivi : main"));
    assert!(shows(
        &rt,
        "git remote add -- 'mirror' 'git@example.org:b.git'"
    ));
    press(&mut rt, rune::Key::Escape);
    assert!(rt.state.remotes.confirm_remove.is_none() && rt.state.remotes.open);
    assert!(remotes(dir.path()).contains(&"mirror".to_owned()));
    click(&mut rt, "rm-remove");
    settle(&mut rt, |s| {
        matches!(&s.remotes.confirm_remove, Some((_, Some(_))))
    });
    click(&mut rt, "rm-remove-confirm");
    settle(&mut rt, |s| idle(s) && s.remotes.remotes.len() == 1);
    assert_eq!(remotes(dir.path()), ["origin"]);
    assert_eq!(
        git(dir.path(), &["symbolic-ref", "--short", "HEAD"]),
        "main"
    );
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("Pour le rétablir"))
    );
}

#[test]
fn removing_a_remote_with_credentials_never_prints_them_in_the_restore_hint() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "origin");
    click(&mut rt, "rm-remove");
    settle(&mut rt, |s| {
        matches!(&s.remotes.confirm_remove, Some((_, Some(_))))
    });
    click(&mut rt, "rm-remove-confirm");
    settle(&mut rt, |s| idle(s) && s.remotes.remotes.len() == 1);
    let notice = rt.state.notice.clone().unwrap_or_default();
    assert!(!notice.contains("s3cr3t"), "{notice}");
    assert!(notice.contains("non affichée"));
}

#[test]
fn escape_closes_the_remote_screen() {
    let dir = repo();
    let mut rt = screen(dir.path());
    press(&mut rt, rune::Key::Escape);
    assert!(!rt.state.remotes.open);
}

#[test]
fn a_removal_confirmed_before_its_impact_is_known_does_nothing() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "mirror");
    rt.state.remotes.confirm_remove = Some(("mirror".to_owned(), None));
    rt.state.remote_remove();
    for _ in 0..6 {
        step(&mut rt);
    }
    assert!(!rt.state.remotes.busy);
    assert_eq!(remotes(dir.path()), ["mirror", "origin"]);
}

#[test]
fn a_token_used_as_the_user_name_is_masked_everywhere_too() {
    let dir = repo();
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "tokened",
            "https://ghp_t0kenvalue@github.com/o/r.git",
        ],
    );
    let mut rt = screen(dir.path());
    pick(&mut rt, "tokened");
    click(&mut rt, "rm-remove");
    settle(&mut rt, |s| {
        matches!(&s.remotes.confirm_remove, Some((_, Some(_))))
    });
    for element in rt.elements() {
        if let Some(text) = element.text.as_deref() {
            assert!(!text.contains("t0kenvalue"), "leaked on screen: {text}");
        }
    }
    click(&mut rt, "rm-remove-confirm");
    settle(&mut rt, |s| idle(s) && s.remotes.remotes.len() == 2);
    let notice = rt.state.notice.clone().unwrap_or_default();
    assert!(!notice.contains("t0kenvalue"), "{notice}");
}
