use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use gitpoulpe_git::Git;
use rune::{Runtime, prefs::Prefs};

use super::{click_at, rect_of, runtime, step};
use crate::overview::RowState;
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
    for _ in 0..600 {
        step(rt);
        if until(&rt.state) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("never saw {what}");
}

fn repo(prefix: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\n").unwrap();
    std::fs::write(p.join("b.txt"), "two\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    dir
}

fn dirty(dir: &Path) {
    std::fs::write(dir.join("a.txt"), "one\nstaged\n").unwrap();
    git(dir, &["add", "a.txt"]);
    std::fs::write(dir.join("b.txt"), "two\nlocal\n").unwrap();
    std::fs::write(dir.join("loose.txt"), "u\n").unwrap();
}

fn text(path: &Path) -> String {
    path.canonicalize().unwrap().to_str().unwrap().to_owned()
}

fn app_with(dir: &Path, git_program: Option<PathBuf>) -> Runtime<State> {
    let mut state = State::new(None, Prefs::memory());
    if let Some(program) = git_program {
        state.engine_git =
            Some(Git::with_program(program).with_env("GIT_CONFIG_GLOBAL", "/dev/null"));
    }
    state.setup(Arc::new(|| {}));
    state.open_blocking(dir.to_path_buf(), std::time::Duration::from_secs(20));
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    rt
}

fn ready(rt: &Runtime<State>, path: &Path) -> Option<gitpoulpe_git::RepoOverview> {
    match rt.state.overview.rows.get(&text(path)).map(|r| &r.state) {
        Some(RowState::Ready(o)) => Some(o.clone()),
        _ => None,
    }
}

fn failed(rt: &Runtime<State>, path: &Path) -> Option<String> {
    match rt.state.overview.rows.get(&text(path)).map(|r| &r.state) {
        Some(RowState::Failed(m)) => Some(m.clone()),
        _ => None,
    }
}

#[test]
fn every_state_names_its_repository_and_one_failure_never_spreads() {
    let (a, b, c, d) = (repo("ov-a-"), repo("ov-b-"), repo("ov-c-"), repo("ov-d-"));
    dirty(b.path());
    let mut rt = app_with(a.path(), None);
    for dir in [b.path(), c.path(), d.path()] {
        rt.state.favorites_add_path(dir.to_path_buf(), None);
    }
    let moved = tempfile::tempdir().unwrap();
    let gone = d.path().to_path_buf();
    std::fs::remove_dir_all(c.path().join(".git")).unwrap();
    let d_text = text(&gone);
    std::fs::rename(&gone, moved.path().join("moved")).unwrap();
    rt.state.open_overview();
    wait_for(&mut rt, "every row settled", |s| {
        s.overview
            .rows
            .values()
            .all(|r| r.state != RowState::Loading)
            && s.overview.rows.len() == 4
    });
    let clean = ready(&rt, a.path()).unwrap();
    assert_eq!((clean.staged, clean.unstaged, clean.untracked), (0, 0, 0));
    let changed = ready(&rt, b.path()).unwrap();
    assert_eq!(
        (changed.staged, changed.unstaged, changed.untracked),
        (1, 1, 1)
    );
    assert!(failed(&rt, c.path()).unwrap().contains("pas un dépôt Git"));
    let missing = rt.state.overview.rows.get(&d_text).unwrap();
    assert!(matches!(&missing.state, RowState::Failed(m) if m.contains("introuvable")));
    for step_ in 0..20 {
        let _ = step_;
        step(&mut rt);
    }
    assert!(shows(&rt, &text(a.path())) && shows(&rt, &text(b.path())));
    assert!(shows(&rt, "1 indexé(s) · 1 modifié(s) · 1 non suivi(s)"));
    assert!(shows(&rt, "0 indexé(s) · 0 modifié(s) · 0 non suivi(s)"));
    assert_eq!(super::head_context(&rt, "ov-head"), "4 dépôts suivis sur 4");
    assert!(shows(&rt, "dépôt ouvert") && shows(&rt, "favori"));
}

#[test]
fn opening_a_row_joins_its_repository_with_the_saved_position() {
    let a = repo("ov-a-");
    let b = repo("ov-b-");
    dirty(b.path());
    let mut rt = app_with(a.path(), None);
    rt.state.workspaces.new_name = rune::TextEdit::new("Deux");
    rt.state.open_workspaces();
    rt.state.workspace_create();
    rt.state.workspace_add_path(b.path().to_path_buf());
    let id = rt.state.workspaces.selected.unwrap();
    rt.state.workspaces.book.active = Some(id);
    let stored = rt.state.workspaces.book.items[0].repos[0].path.clone();
    rt.state
        .workspaces
        .book
        .remember(id, &stored, "staged", Some("a.txt".to_owned()));
    rt.state.open_overview();
    wait_for(&mut rt, "rows", |s| {
        s.overview.rows.len() == 2
            && s.overview
                .rows
                .values()
                .all(|r| r.state != RowState::Loading)
    });
    for _ in 0..20 {
        step(&mut rt);
    }
    assert!(shows(&rt, "position enregistrée : vue indexés, a.txt"));
    let index = rt
        .state
        .overview_candidates()
        .iter()
        .position(|c| c.path == text(b.path()))
        .unwrap();
    click(&mut rt, &format!("ov-open{index}"));
    wait_for(&mut rt, "repository B with its view", |s| {
        s.repo
            .as_ref()
            .is_some_and(|r| r.path.ends_with(b.path().file_name().unwrap()))
            && s.view == ViewKind::Staged
            && s.current.is_some()
    });
    assert!(!rt.state.overview.open);
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

fn slow_git(base: &Path) -> PathBuf {
    let script = base.join("slow-git.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\ncase \"$PWD\" in *slow*) sleep 30;; esac\nexec git \"$@\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[test]
fn a_slow_repository_neither_blocks_the_fast_ones_nor_waits_forever() {
    let tools = tempfile::tempdir().unwrap();
    let (a, b, c) = (repo("ov-fast-a-"), repo("ov-fast-b-"), repo("ov-slow-"));
    let mut rt = app_with(a.path(), Some(slow_git(tools.path())));
    rt.state.favorites_add_path(b.path().to_path_buf(), None);
    rt.state.favorites_add_path(c.path().to_path_buf(), None);
    rt.state.overview.timeout_ms = 1500;
    rt.state.open_overview();
    wait_for(&mut rt, "the fast repositories", |s| {
        s.overview
            .rows
            .values()
            .filter(|r| matches!(r.state, RowState::Ready(_)))
            .count()
            == 2
    });
    assert_eq!(
        rt.state.overview.rows.get(&text(c.path())).unwrap().state,
        RowState::Loading,
        "the slow one is still being read when the others are done"
    );
    wait_for(&mut rt, "the slow repository's timeout", |s| {
        matches!(
            s.overview.rows.get(&text(c.path())).map(|r| &r.state),
            Some(RowState::Failed(_))
        )
    });
    let message = failed(&rt, c.path()).unwrap();
    assert!(
        message.contains("délai dépassé") && message.contains("les autres ne sont pas bloqués")
    );
    assert!(ready(&rt, a.path()).is_some() && ready(&rt, b.path()).is_some());
}

#[test]
fn quick_selection_changes_never_let_a_stale_answer_through() {
    let tools = tempfile::tempdir().unwrap();
    let (a, b) = (repo("ov-fast-"), repo("ov-slow-"));
    let mut rt = app_with(a.path(), Some(slow_git(tools.path())));
    rt.state.favorites_add_path(b.path().to_path_buf(), None);
    rt.state.overview.timeout_ms = 20_000;
    rt.state.open_overview();
    let slow = text(b.path());
    wait_for(&mut rt, "the fast repository", |s| {
        s.overview
            .rows
            .values()
            .any(|r| matches!(r.state, RowState::Ready(_)))
    });
    let first_job = rt.state.overview.rows.get(&slow).unwrap().job;
    rt.state.overview_toggle(&slow);
    assert!(!rt.state.overview.rows.contains_key(&slow));
    let late = gitpoulpe_git::RepoOverview::default();
    assert!(
        !rt.state
            .apply_overview_repo(slow.clone(), first_job, Ok(late.clone()))
    );
    rt.state.overview_toggle(&slow);
    let second_job = rt.state.overview.rows.get(&slow).unwrap().job;
    assert!(second_job > first_job);
    assert!(
        !rt.state
            .apply_overview_repo(slow.clone(), first_job, Ok(late.clone()))
    );
    assert_eq!(
        rt.state.overview.rows.get(&slow).unwrap().state,
        RowState::Loading
    );
    assert!(
        rt.state
            .apply_overview_repo(slow.clone(), second_job, Ok(late))
    );
    rt.state.overview_select_none();
    assert!(rt.state.overview.rows.is_empty() && rt.state.overview.selected.is_empty());
    rt.state.overview_select_all();
    assert_eq!(rt.state.overview.selected.len(), 2);
    wait_for(&mut rt, "the fast row again", |s| {
        s.overview
            .rows
            .values()
            .any(|r| matches!(r.state, RowState::Ready(_)))
    });
}

#[test]
fn the_palette_offers_the_overview_and_an_empty_one_explains_itself() {
    let command = crate::palette::COMMANDS
        .iter()
        .find(|c| c.id == "overview")
        .unwrap();
    let mut rt = runtime(State::new(None, Prefs::memory()));
    assert!((command.available)(&rt.state));
    rt.state.open_overview();
    step(&mut rt);
    assert!(shows(&rt, "Aucun dépôt à afficher"));
}

#[test]
fn a_repository_in_two_workspaces_opens_with_the_position_of_the_active_one() {
    let a = repo("ov-a-");
    let b = repo("ov-b-");
    dirty(b.path());
    let mut rt = app_with(a.path(), None);
    rt.state.open_workspaces();
    let mut ids = Vec::new();
    for (name, view) in [("Premier", "staged"), ("Second", "unstaged")] {
        rt.state.workspaces.new_name = rune::TextEdit::new(name);
        rt.state.workspace_create();
        rt.state.workspace_add_path(b.path().to_path_buf());
        let id = rt.state.workspaces.selected.unwrap();
        let stored = rt.state.workspaces.book.get(id).unwrap().repos[0]
            .path
            .clone();
        rt.state
            .workspaces
            .book
            .remember(id, &stored, view, Some("b.txt".to_owned()));
        ids.push(id);
    }
    rt.state.workspaces.book.active = Some(ids[1]);
    rt.state.open_overview();
    wait_for(&mut rt, "rows", |s| {
        s.overview.rows.len() == 2
            && s.overview
                .rows
                .values()
                .all(|r| r.state != RowState::Loading)
    });
    rt.state.overview_open(&text(b.path()));
    wait_for(&mut rt, "repository B", |s| {
        s.repo
            .as_ref()
            .is_some_and(|r| r.path.ends_with(b.path().file_name().unwrap()))
            && s.current.is_some()
    });
    assert_eq!(rt.state.view, ViewKind::Unstaged);
    assert_eq!(rt.state.workspaces.selected, Some(ids[1]));
}
