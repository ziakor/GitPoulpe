use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use gitpoulpe_git::Git;
use rune::Runtime;

use super::{click_at, rect_of, runtime, step};
use crate::state::State;

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
    panic!("the screen never settled");
}

fn click(rt: &mut Runtime<State>, key: &str) {
    if key.starts_with("sm-") && !key.starts_with("sm-row") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("sm-panel", target));
        for _ in 0..8 {
            step(rt);
        }
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn engine_git() -> Git {
    Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_ALLOW_PROTOCOL", "file")
}

fn sg(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_ALLOW_PROTOCOL", "file")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn super_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let area = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(area.path()).unwrap();
    let sub = root.join("sub-origin");
    std::fs::create_dir(&sub).unwrap();
    sg(&sub, &["init", "-q", "-b", "main"]);
    std::fs::write(sub.join("lib.txt"), "v1\n").unwrap();
    sg(&sub, &["add", "-A"]);
    sg(&sub, &["commit", "-q", "-m", "sub one"]);
    let origin = root.join("super-origin");
    std::fs::create_dir(&origin).unwrap();
    sg(&origin, &["init", "-q", "-b", "main"]);
    sg(
        &origin,
        &[
            "submodule",
            "add",
            "-q",
            &sub.display().to_string(),
            "libs/sub",
        ],
    );
    sg(&origin, &["commit", "-q", "-m", "add submodule"]);
    sg(
        &root,
        &["clone", "-q", &origin.display().to_string(), "work"],
    );
    let work = root.join("work");
    (area, work)
}

fn screen(dir: &Path) -> Runtime<State> {
    let mut state = State::new(None, rune::prefs::Prefs::memory());
    state.engine_git = Some(engine_git());
    state.setup(Arc::new(|| {}));
    state.open_blocking(dir.to_path_buf(), Duration::from_secs(20));
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    click(&mut rt, "branches-submodules");
    settle(&mut rt, |s| s.submodules.open && !s.submodules.loading);
    click(&mut rt, "smk0");
    settle(&mut rt, |s| s.submodules.selected.is_some());
    rt
}

#[test]
fn an_uninitialised_submodule_is_initialised_then_put_on_its_expected_commit() {
    let (_area, work) = super_repo();
    let mut rt = screen(&work);
    assert!(shows(&rt, "non initialisé"));
    assert!(rect_of(&rt, "sm-update").is_none());
    click(&mut rt, "sm-init");
    settle(&mut rt, |s| {
        s.submodules.running.is_none() && s.submodules.done.is_some()
    });
    assert!(shows(&rt, "initialisé"));
    click(&mut rt, "sm-update");
    settle(&mut rt, |s| {
        s.submodules.running.is_none()
            && s.submodules
                .done
                .as_deref()
                .is_some_and(|d| d.contains("commit attendu"))
    });
    assert!(shows(&rt, "sur le commit attendu"));
    assert!(work.join("libs/sub/lib.txt").is_file());
}

#[test]
fn unsaved_submodule_commits_need_a_confirmation_and_stay_recoverable() {
    let (_area, work) = super_repo();
    let mut rt = screen(&work);
    click(&mut rt, "sm-init");
    settle(&mut rt, |s| s.submodules.done.is_some());
    click(&mut rt, "sm-update");
    settle(&mut rt, |s| {
        s.submodules.running.is_none()
            && s.submodules
                .done
                .as_deref()
                .is_some_and(|d| d.contains("commit attendu"))
    });
    let inner = work.join("libs/sub");
    std::fs::write(inner.join("lib.txt"), "mine\n").unwrap();
    sg(&inner, &["commit", "-q", "-am", "local work"]);
    let local = sg(&inner, &["rev-parse", "HEAD"]);
    click(&mut rt, "smk0");
    settle(&mut rt, |s| s.submodules.selected.is_some());
    click(&mut rt, "sm-update");
    settle(&mut rt, |s| {
        s.submodules.running.is_none() && s.submodules.needs_decision.is_some()
    });
    assert_eq!(sg(&inner, &["rev-parse", "HEAD"]), local);
    assert!(rect_of(&rt, "sm-confirm").is_some());
    click(&mut rt, "sm-confirm");
    settle(&mut rt, |s| {
        s.submodules.running.is_none() && s.submodules.needs_decision.is_none()
    });
    assert_ne!(sg(&inner, &["rev-parse", "HEAD"]), local);
    let markers = sg(&inner, &["for-each-ref", "refs/gitpoulpe/recovery/"]);
    assert!(markers.contains(&local), "{markers}");
}

#[test]
fn a_submodule_can_be_opened_as_its_own_repository() {
    let (_area, work) = super_repo();
    let mut rt = screen(&work);
    click(&mut rt, "sm-init");
    settle(&mut rt, |s| s.submodules.done.is_some());
    click(&mut rt, "sm-update");
    settle(&mut rt, |s| {
        s.submodules.running.is_none()
            && s.submodules
                .done
                .as_deref()
                .is_some_and(|d| d.contains("commit attendu"))
    });
    click(&mut rt, "sm-open");
    settle(&mut rt, |s| {
        s.repo.as_ref().is_some_and(|r| r.name == "sub")
    });
}
