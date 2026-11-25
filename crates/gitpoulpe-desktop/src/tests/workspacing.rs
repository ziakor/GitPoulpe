use std::path::{Path, PathBuf};
use std::process::Command;

use gitpoulpe_core::WorkspaceBook;
use rune::{Runtime, TextEdit};

use super::{click_at, rect_of, runtime, step};
use crate::state::{State, ViewKind};

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
    assert!(output.status.success(), "{args:?}");
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn wait_for(rt: &mut Runtime<State>, what: &str, until: impl Fn(&State) -> bool) {
    for _ in 0..500 {
        step(rt);
        if until(&rt.state) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("never saw {what}");
}

fn repo(name: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix(name).tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\n").unwrap();
    std::fs::write(p.join("b.txt"), "other\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    std::fs::write(p.join("a.txt"), "one\nstaged\n").unwrap();
    git(p, &["add", "a.txt"]);
    std::fs::write(p.join("b.txt"), "other\nlocal\n").unwrap();
    dir
}

fn app(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    rt
}

fn workspace_of(rt: &mut Runtime<State>, name: &str) -> u64 {
    rt.state.open_workspaces();
    step(rt);
    rt.state.workspaces.new_name = TextEdit::new(name);
    step(rt);
    click(rt, "ws-create");
    step(rt);
    rt.state.workspaces.selected.unwrap()
}

fn text_of(path: &Path) -> String {
    path.canonicalize().unwrap().to_str().unwrap().to_owned()
}

#[test]
fn a_workspace_is_created_renamed_and_deleted_after_a_confirmation_without_touching_any_file() {
    let a = repo("ws-a-");
    let mut rt = app(a.path());
    workspace_of(&mut rt, "Travail");
    assert!(shows(&rt, "Travail · 0 dépôt"));
    click(&mut rt, "ws-add-current");
    step(&mut rt);
    assert!(shows(&rt, &text_of(a.path())));
    rt.state.workspaces.rename = TextEdit::new("Boulot");
    step(&mut rt);
    click(&mut rt, "ws-rename");
    step(&mut rt);
    assert!(shows(&rt, "Boulot · 1 dépôt"));
    rt.state.workspaces.new_name = TextEdit::new("boulot");
    step(&mut rt);
    click(&mut rt, "ws-create");
    step(&mut rt);
    assert!(shows(&rt, "existe déjà"));
    click(&mut rt, "ws-delete");
    step(&mut rt);
    assert!(shows(&rt, "aucun fichier de dépôt n'est touché"));
    assert_eq!(rt.state.workspaces.book.items.len(), 1);
    click(&mut rt, "ws-cancel-delete");
    step(&mut rt);
    click(&mut rt, "ws-delete");
    step(&mut rt);
    click(&mut rt, "ws-confirm-delete");
    step(&mut rt);
    assert!(rt.state.workspaces.book.items.is_empty());
    assert!(a.path().join(".git").exists() && a.path().join("a.txt").exists());
}

#[test]
fn only_real_repositories_are_added_and_only_once() {
    let a = repo("ws-a-");
    let plain = tempfile::tempdir().unwrap();
    let mut rt = app(a.path());
    workspace_of(&mut rt, "W");
    rt.state.workspace_add_path(plain.path().to_path_buf());
    step(&mut rt);
    assert!(shows(&rt, "n'est pas un dépôt Git"));
    rt.state.workspace_add_path(a.path().to_path_buf());
    rt.state.workspace_add_path(a.path().to_path_buf());
    step(&mut rt);
    assert!(shows(&rt, "est déjà dans cet espace"));
    assert_eq!(rt.state.workspaces.book.items[0].repos.len(), 1);
}

#[test]
fn restarting_offline_restores_the_workspace_with_its_repositories_and_saved_view() {
    let a = repo("ws-a-");
    let b = repo("ws-b-");
    let mut rt = app(a.path());
    workspace_of(&mut rt, "Deux");
    rt.state.workspace_add_path(a.path().to_path_buf());
    rt.state.workspace_add_path(b.path().to_path_buf());
    step(&mut rt);
    let id = rt.state.workspaces.selected.unwrap();
    rt.state.workspaces.book.active = Some(id);
    rt.state.workspace_open_repo(text_of(b.path()));
    wait_for(&mut rt, "repository B", |s| {
        s.repo
            .as_ref()
            .is_some_and(|r| r.path.ends_with(b.path().file_name().unwrap()))
    });
    rt.state.set_view(ViewKind::Staged);
    wait_for(&mut rt, "a current staged file", |s| {
        s.view == ViewKind::Staged && s.current.and_then(|p| s.entry_at(p)).is_some()
    });
    rt.state.open_workspaces();
    step(&mut rt);
    let saved = rt.state.workspaces.book.clone();
    assert_eq!(saved.items[0].repos[1].view, "staged");
    assert_eq!(saved.items[0].repos[1].file.as_deref(), Some("a.txt"));

    let mut prefs = rune::prefs::Prefs::memory();
    prefs
        .set(crate::workspaces::WORKSPACES_KEY, &saved)
        .unwrap();
    let mut fresh = State::new(None, prefs);
    fresh.setup(std::sync::Arc::new(|| {}));
    let mut rt = runtime(fresh);
    rt.on_key = Some(crate::keys::on_key);
    rt.state.open_workspaces();
    step(&mut rt);
    assert!(shows(&rt, "Deux (actif) · 2 dépôts"));
    click(&mut rt, "ws-restore");
    wait_for(&mut rt, "the restored repository", |s| {
        s.repo.is_some() && s.view == ViewKind::Staged && s.current.is_some()
    });
    assert!(
        rt.state
            .repo
            .as_ref()
            .unwrap()
            .path
            .ends_with(b.path().file_name().unwrap())
    );
    let current = rt
        .state
        .current
        .and_then(|p| rt.state.entry_at(p))
        .unwrap()
        .1
        .path
        .clone();
    assert_eq!(current, PathBuf::from("a.txt"));
}

#[test]
fn removing_a_repository_from_a_workspace_deletes_nothing() {
    let a = repo("ws-a-");
    let mut rt = app(a.path());
    workspace_of(&mut rt, "W");
    rt.state.workspace_add_path(a.path().to_path_buf());
    step(&mut rt);
    click(&mut rt, "ws-remove0");
    step(&mut rt);
    assert!(rt.state.workspaces.book.items[0].repos.is_empty());
    assert!(shows(&rt, "aucun fichier n'a été supprimé"));
    assert!(a.path().join("a.txt").exists() && a.path().join(".git").exists());
}

#[test]
fn a_moved_or_broken_repository_is_flagged_and_repairable_without_blocking_the_workspace() {
    let a = repo("ws-a-");
    let b = repo("ws-b-");
    let mut rt = app(a.path());
    workspace_of(&mut rt, "W");
    rt.state.workspace_add_path(b.path().to_path_buf());
    rt.state.workspace_add_path(a.path().to_path_buf());
    step(&mut rt);
    let old_b = text_of(b.path());
    let moved = tempfile::tempdir().unwrap();
    let new_b = moved.path().join("moved-b");
    std::fs::rename(b.path(), &new_b).unwrap();
    std::mem::forget(b);
    rt.state.open_workspaces();
    step(&mut rt);
    assert!(shows(&rt, "introuvable"));
    assert!(rect_of(&rt, "ws-repair0").is_some() && rect_of(&rt, "ws-repair1").is_none());
    click(&mut rt, "ws-open0");
    step(&mut rt);
    assert!(shows(&rt, "utilisez « Réparer »"));
    rt.state.workspace_restore();
    wait_for(&mut rt, "the reachable repository", |s| {
        s.repo
            .as_ref()
            .is_some_and(|r| r.path.ends_with(a.path().file_name().unwrap()))
    });
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("1 dépôt de l'espace est introuvable"))
    );
    rt.state.open_workspaces();
    step(&mut rt);
    rt.state.workspace_repair_path(&old_b, new_b.clone());
    step(&mut rt);
    assert!(shows(&rt, "Association réparée"));
    for _ in 0..50 {
        if rect_of(&rt, "ws-repair0").is_none() {
            break;
        }
        step(&mut rt);
    }
    assert!(rect_of(&rt, "ws-repair0").is_none());
    std::fs::remove_dir_all(new_b.join(".git")).unwrap();
    rt.state.open_workspaces();
    step(&mut rt);
    assert!(shows(&rt, "n'est plus un dépôt Git"));
    rt.state
        .workspace_repair_path(&text_of(&new_b), moved.path().to_path_buf());
    step(&mut rt);
    assert!(shows(&rt, "n'est pas un dépôt Git"));
}

#[test]
fn leaving_a_repository_remembers_its_view_for_the_next_visit() {
    let a = repo("ws-a-");
    let b = repo("ws-b-");
    let mut rt = app(a.path());
    workspace_of(&mut rt, "W");
    rt.state.workspace_add_path(a.path().to_path_buf());
    rt.state.workspace_add_path(b.path().to_path_buf());
    let id = rt.state.workspaces.selected.unwrap();
    rt.state.workspaces.book.active = Some(id);
    rt.state.workspace_open_repo(text_of(a.path()));
    wait_for(&mut rt, "repository A", |s| {
        s.repo
            .as_ref()
            .is_some_and(|r| r.path.ends_with(a.path().file_name().unwrap()))
    });
    rt.state.set_view(ViewKind::Staged);
    wait_for(&mut rt, "a staged file", |s| {
        s.view == ViewKind::Staged && s.current.is_some()
    });
    rt.state.workspaces.selected = Some(id);
    rt.state.workspace_open_repo(text_of(b.path()));
    wait_for(&mut rt, "repository B", |s| {
        s.repo
            .as_ref()
            .is_some_and(|r| r.path.ends_with(b.path().file_name().unwrap()))
    });
    assert_eq!(rt.state.view, ViewKind::Unstaged);
    rt.state.open_workspaces();
    rt.state.workspace_select(id);
    rt.state.workspace_open_repo(text_of(a.path()));
    wait_for(&mut rt, "repository A again", |s| {
        s.repo
            .as_ref()
            .is_some_and(|r| r.path.ends_with(a.path().file_name().unwrap()))
            && s.view == ViewKind::Staged
    });
}

#[test]
fn the_palette_opens_the_workspaces() {
    let a = repo("ws-a-");
    let mut rt = app(a.path());
    rt.state.commands.open = false;
    let command = crate::palette::COMMANDS
        .iter()
        .find(|c| c.id == "workspaces")
        .unwrap();
    assert!((command.available)(&rt.state));
    let book: WorkspaceBook = rt.state.workspaces.book.clone();
    assert!(book.items.is_empty());
    rt.state.open_workspaces();
    step(&mut rt);
    assert!(shows(&rt, "Créez un espace de travail"));
}
