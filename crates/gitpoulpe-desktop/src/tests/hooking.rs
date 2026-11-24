use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use rune::Runtime;

use super::{click_at, press, rect_of, runtime, step};
use crate::state::State;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
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
    for _ in 0..100 {
        step(rt);
        if until(&rt.state) {
            break;
        }
    }
    for _ in 0..12 {
        step(rt);
    }
    for _ in 0..100 {
        if until(&rt.state) {
            return;
        }
        step(rt);
    }
    panic!("the screen never settled: notice={:?}", rt.state.notice);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    if key.starts_with("hk-") && !key.starts_with("hk-row") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("hk-panel", target));
        for _ in 0..8 {
            step(rt);
        }
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn write_hook(dir: &Path, name: &str, mode: u32) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    path
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "a\n").unwrap();
    git(p, &["add", "-A"]);
    git(
        p,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-q",
            "-m",
            "one",
        ],
    );
    dir
}

fn screen(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    super::menu(&mut rt, "nav-hooks");
    settle(&mut rt, |s| s.hooks.open && !s.hooks.loading);
    rt
}

#[test]
fn the_effective_directory_and_the_active_and_disabled_hooks_are_identifiable() {
    let dir = repo();
    let hooks = dir.path().join(".git/hooks");
    write_hook(&hooks, "pre-commit", 0o755);
    write_hook(&hooks, "commit-msg", 0o644);
    let rt = screen(dir.path());
    assert!(shows(&rt, "Aucun core.hooksPath"));
    assert!(shows(&rt, "actif"));
    assert!(shows(&rt, "désactivé (non exécutable : Git l'ignore)"));
    assert!(shows(&rt, "rien n'est contourné"));
}

#[test]
fn a_custom_hooks_path_is_shown_with_its_scope() {
    let dir = repo();
    write_hook(&dir.path().join(".githooks"), "pre-commit", 0o755);
    git(dir.path(), &["config", "core.hooksPath", ".githooks"]);
    let rt = screen(dir.path());
    assert!(shows(&rt, "core.hooksPath = .githooks (portée : local)"));
    assert!(shows(&rt, "fait partie du projet"));
}

#[test]
fn a_hook_is_disabled_and_enabled_explicitly_in_the_repository_scope() {
    let dir = repo();
    let hook = write_hook(&dir.path().join(".git/hooks"), "pre-commit", 0o755);
    let mut rt = screen(dir.path());
    rt.state.select_hook("pre-commit");
    step(&mut rt);
    click(&mut rt, "hk-toggle");
    settle(&mut rt, |s| {
        !s.hooks.busy && s.notice.as_deref() == Some("Hook pre-commit désactivé.")
    });
    assert_eq!(
        std::fs::metadata(&hook).unwrap().permissions().mode() & 0o111,
        0
    );
    click(&mut rt, "hk-toggle");
    settle(&mut rt, |s| {
        !s.hooks.busy && s.notice.as_deref() == Some("Hook pre-commit activé.")
    });
    assert_eq!(
        std::fs::metadata(&hook).unwrap().permissions().mode() & 0o111,
        0o111
    );
}

#[test]
fn a_shared_directory_asks_for_the_scope_before_anything_changes() {
    let shared = tempfile::tempdir().unwrap();
    let hook = write_hook(shared.path(), "pre-push", 0o755);
    let dir = repo();
    git(
        dir.path(),
        &[
            "config",
            "core.hooksPath",
            &shared.path().display().to_string(),
        ],
    );
    let mut rt = screen(dir.path());
    rt.state.select_hook("pre-push");
    step(&mut rt);
    click(&mut rt, "hk-toggle");
    settle(&mut rt, |s| {
        !s.hooks.busy && s.hooks.confirm_shared.is_some()
    });
    assert!(shows(&rt, "Ce dossier est partagé"));
    assert_eq!(
        std::fs::metadata(&hook).unwrap().permissions().mode() & 0o111,
        0o111
    );
    press(&mut rt, rune::Key::Escape);
    assert!(rt.state.hooks.confirm_shared.is_none() && rt.state.hooks.open);
    click(&mut rt, "hk-toggle");
    settle(&mut rt, |s| s.hooks.confirm_shared.is_some());
    click(&mut rt, "hk-confirm-yes");
    settle(&mut rt, |s| {
        !s.hooks.busy && s.hooks.confirm_shared.is_none()
    });
    assert_eq!(
        std::fs::metadata(&hook).unwrap().permissions().mode() & 0o111,
        0
    );
}
