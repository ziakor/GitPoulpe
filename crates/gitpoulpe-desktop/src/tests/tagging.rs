use std::path::Path;
use std::process::Command;

use rune::{Runtime, TextEdit};

use super::{click_at, press, rect_of, runtime, step};
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
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
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
    !s.tags.busy && !s.tags.loading
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

struct Fixture {
    dir: tempfile::TempDir,
    first: String,
}

fn repo() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Ada Lovelace"]);
    git(p, &["config", "user.email", "ada@example.org"]);
    std::fs::write(p.join("a.txt"), "one\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "first commit"]);
    let first = git(p, &["rev-parse", "HEAD"]);
    std::fs::write(p.join("b.txt"), "two\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "second commit"]);
    git(p, &["branch", "side", &first]);
    git(p, &["tag", "old", &first]);
    git(
        p,
        &[
            "tag",
            "-a",
            "-m",
            "Release notes\n\nsecond paragraph",
            "v1",
            "HEAD",
        ],
    );
    Fixture { dir, first }
}

fn screen(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    click(&mut rt, "branches-tags");
    settle(&mut rt, |s| s.tags.open && idle(s));
    rt
}

fn pick(rt: &mut Runtime<State>, name: &str) {
    rt.state.select_tag(name);
    for _ in 0..6 {
        step(rt);
    }
}

fn tags(dir: &Path) -> Vec<String> {
    git(dir, &["tag", "--list"])
        .lines()
        .map(str::to_owned)
        .collect()
}

fn heads(dir: &Path) -> String {
    git(
        dir,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads",
        ],
    )
}

#[test]
fn the_tag_screen_lists_tags_and_reads_the_details_of_an_annotated_one() {
    let f = repo();
    let mut rt = screen(f.dir.path());
    let listed: Vec<String> = rt.state.tags.tags.iter().map(|t| t.name.clone()).collect();
    assert_eq!(listed, tags(f.dir.path()));
    assert!(shows(&rt, "léger") && shows(&rt, "annoté"));
    pick(&mut rt, "v1");
    assert!(shows(&rt, "Tag annoté"));
    assert!(shows(&rt, "Ada Lovelace <ada@example.org>"));
    assert!(shows(&rt, "second paragraph"));
    pick(&mut rt, "old");
    assert!(shows(&rt, "Tag léger"));
    assert!(shows(&rt, &f.first));
}

#[test]
fn a_tag_created_from_the_history_points_at_that_commit_not_at_head() {
    let f = repo();
    let mut rt = runtime(super::opened(f.dir.path()));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "history");
    settle(&mut rt, |s| s.history.status == HistoryStatus::Ready);
    let index = rt
        .state
        .history
        .commits
        .iter()
        .position(|c| c.oid == f.first)
        .unwrap();
    rt.state.select_commit(index);
    for _ in 0..4 {
        step(&mut rt);
    }
    click(&mut rt, "tag-commit");
    settle(&mut rt, |s| s.tags.open && idle(s));
    assert!(shows(&rt, "Nouveau tag sur le commit"));

    rt.state.tags.new_name = TextEdit::new("at-first");
    click(&mut rt, "tag-create-light");
    settle(&mut rt, |s| {
        idle(s) && s.tags.tags.iter().any(|t| t.name == "at-first")
    });
    assert_eq!(
        git(f.dir.path(), &["rev-parse", "refs/tags/at-first"]),
        f.first
    );

    rt.state.tags.new_name = TextEdit::new("notes-first");
    rt.state.tags.message = TextEdit::new("annotated on the first commit");
    click(&mut rt, "tag-create-annotated");
    settle(&mut rt, |s| {
        idle(s) && s.tags.tags.iter().any(|t| t.name == "notes-first")
    });
    assert_eq!(
        git(
            f.dir.path(),
            &["rev-parse", "refs/tags/notes-first^{commit}"]
        ),
        f.first
    );
    assert_eq!(
        git(f.dir.path(), &["cat-file", "-t", "refs/tags/notes-first"]),
        "tag"
    );
    assert!(
        rt.state
            .journal()
            .iter()
            .any(|e| e.action == "tag-create" && e.detail == "notes-first")
    );
}

#[test]
fn invalid_duplicate_and_message_less_requests_are_refused_with_a_reason() {
    let f = repo();
    let before = tags(f.dir.path());
    let old = git(f.dir.path(), &["rev-parse", "refs/tags/old"]);
    let mut rt = screen(f.dir.path());

    rt.state.tags.new_name = TextEdit::new("bad name");
    click(&mut rt, "tag-create-light");
    settle(&mut rt, |s| idle(s) && s.notice.is_some());
    assert!(
        rt.state
            .notice
            .as_deref()
            .unwrap()
            .contains("nom de tag invalide")
    );

    rt.state.notice = None;
    rt.state.tags.new_name = TextEdit::new("old");
    click(&mut rt, "tag-create-light");
    settle(&mut rt, |s| idle(s) && s.notice.is_some());
    assert!(
        rt.state
            .notice
            .as_deref()
            .unwrap()
            .contains("porte déjà ce nom")
    );
    assert_eq!(git(f.dir.path(), &["rev-parse", "refs/tags/old"]), old);

    rt.state.notice = None;
    rt.state.tags.new_name = TextEdit::new("fresh");
    rt.state.tags.message = TextEdit::new("");
    click(&mut rt, "tag-create-annotated");
    for _ in 0..6 {
        step(&mut rt);
    }
    assert!(
        rt.state
            .notice
            .as_deref()
            .unwrap()
            .contains("besoin d'un message")
    );
    assert_eq!(tags(f.dir.path()), before);
}

#[test]
fn deleting_a_tag_names_it_asks_first_and_leaves_the_branches_alone() {
    let f = repo();
    let branches = heads(f.dir.path());
    let object = git(f.dir.path(), &["rev-parse", "refs/tags/v1"]);
    let mut rt = screen(f.dir.path());
    pick(&mut rt, "v1");
    click(&mut rt, "tag-delete");
    for _ in 0..6 {
        step(&mut rt);
    }
    assert!(shows(&rt, "Supprimer le tag « v1 »"));
    assert!(shows(&rt, &format!("git update-ref refs/tags/v1 {object}")));
    assert!(
        tags(f.dir.path()).contains(&"v1".to_owned()),
        "nothing is deleted before the confirmation"
    );

    click(&mut rt, "tag-delete-cancel");
    for _ in 0..6 {
        step(&mut rt);
    }
    assert!(tags(f.dir.path()).contains(&"v1".to_owned()));
    assert!(rect_of(&rt, "tag-delete").is_some());

    click(&mut rt, "tag-delete");
    for _ in 0..6 {
        step(&mut rt);
    }
    click(&mut rt, "tag-delete-confirm");
    settle(&mut rt, |s| {
        idle(s) && !s.tags.tags.iter().any(|t| t.name == "v1")
    });
    assert!(!tags(f.dir.path()).contains(&"v1".to_owned()));
    assert_eq!(heads(f.dir.path()), branches);
    assert!(
        rt.state
            .notice
            .as_deref()
            .unwrap()
            .contains("Tag v1 supprimé")
    );
    let entry = rt
        .state
        .journal()
        .into_iter()
        .find(|e| e.action == "tag-delete")
        .unwrap();
    assert_eq!(entry.before.as_deref(), Some(object.as_str()));
}

#[test]
fn a_tag_leads_to_its_commit_and_escape_backs_out_step_by_step() {
    let f = repo();
    let mut rt = screen(f.dir.path());
    pick(&mut rt, "old");
    click(&mut rt, "tag-inspect");
    settle(&mut rt, |s| {
        s.view == ViewKind::Commit && s.inspected.is_some()
    });
    assert!(!rt.state.tags.open);
    assert_eq!(
        rt.state.inspected.as_ref().unwrap().detail.commit.oid,
        f.first
    );

    let mut rt = screen(f.dir.path());
    pick(&mut rt, "old");
    click(&mut rt, "tag-delete");
    for _ in 0..4 {
        step(&mut rt);
    }
    press(&mut rt, rune::Key::Escape);
    assert!(rt.state.tags.confirm_delete.is_none() && rt.state.tags.open);
    press(&mut rt, rune::Key::Escape);
    assert!(!rt.state.tags.open);
    assert!(tags(f.dir.path()).contains(&"old".to_owned()));
}
