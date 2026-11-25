use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use gitpoulpe_git::Git;
use rune::{Runtime, TextEdit};

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
    if key.starts_with("gf-") && !key.starts_with("gf-row") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("gf-panel", target));
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
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com")
}

fn gg(dir: &Path, args: &[&str]) -> String {
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
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn repo(with_develop: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    gg(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "base\n").unwrap();
    gg(p, &["add", "-A"]);
    gg(p, &["commit", "-q", "-m", "base"]);
    if with_develop {
        gg(p, &["branch", "develop"]);
    }
    dir
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
    super::menu(&mut rt, "nav-gitflow");
    settle(&mut rt, |s| s.gitflow.open && s.gitflow.view.is_some());
    rt
}

fn type_name(rt: &mut Runtime<State>, name: &str) {
    rt.state.gitflow.name = TextEdit::new(name);
    step(rt);
}

#[test]
fn the_default_convention_is_shown_unsaved_and_saving_announces_the_develop_creation() {
    let dir = repo(false);
    let mut rt = screen(dir.path());
    assert!(shows(&rt, "Convention par défaut"));
    assert!(shows(&rt, "n'existe pas"));
    assert!(
        gg(dir.path(), &["config", "--local", "--list"])
            .lines()
            .all(|l| !l.starts_with("gitflow."))
    );
    click(&mut rt, "gf-save");
    settle(&mut rt, |s| s.gitflow.save_confirm.is_some());
    assert!(shows(&rt, "La branche develop sera créée à partir de main"));
    assert_eq!(
        gg(dir.path(), &["branch", "--format=%(refname:short)"]),
        "main"
    );
    click(&mut rt, "gf-save-confirm");
    settle(&mut rt, |s| {
        !s.gitflow.busy && s.gitflow.view.as_ref().is_some_and(|v| v.saved)
    });
    assert!(shows(&rt, "Convention enregistrée"));
    assert_eq!(
        gg(dir.path(), &["config", "--local", "gitflow.branch.develop"]),
        "develop"
    );
    assert!(gg(dir.path(), &["branch", "--format=%(refname:short)"]).contains("develop"));
}

#[test]
fn starting_a_feature_announces_its_steps_and_only_then_creates_the_branch() {
    let dir = repo(true);
    let mut rt = screen(dir.path());
    type_name(&mut rt, "login");
    click(&mut rt, "gf-start-feature");
    settle(&mut rt, |s| s.gitflow.plan.is_some());
    assert!(shows(
        &rt,
        "créer la branche feature/login à partir de develop"
    ));
    assert!(!gg(dir.path(), &["branch"]).contains("feature/login"));
    click(&mut rt, "gf-confirm");
    settle(&mut rt, |s| !s.gitflow.busy && s.gitflow.done.is_some());
    assert_eq!(
        gg(dir.path(), &["symbolic-ref", "--short", "HEAD"]),
        "feature/login"
    );
    assert!(shows(&rt, "feature/login"));
}

#[test]
fn finishing_a_release_shows_the_merge_and_tag_before_running_them() {
    let dir = repo(true);
    let p = dir.path();
    gg(p, &["checkout", "-q", "-b", "release/1.0.0", "develop"]);
    std::fs::write(p.join("r.txt"), "r\n").unwrap();
    gg(p, &["add", "-A"]);
    gg(p, &["commit", "-q", "-m", "release work"]);
    let mut rt = screen(p);
    click(&mut rt, "gfk0");
    settle(&mut rt, |s| s.gitflow.selected.is_some());
    click(&mut rt, "gf-finish");
    settle(&mut rt, |s| s.gitflow.plan.is_some());
    assert!(shows(&rt, "créer le tag annoté 1.0.0 sur main"));
    assert!(gg(p, &["tag"]).is_empty());
    click(&mut rt, "gf-confirm");
    settle(&mut rt, |s| !s.gitflow.busy && s.gitflow.done.is_some());
    assert_eq!(gg(p, &["tag"]), "1.0.0");
    assert!(!gg(p, &["branch"]).contains("release/1.0.0"));
}

#[test]
fn a_conflict_while_finishing_opens_the_resolution_screen_and_explains_the_resume() {
    let dir = repo(true);
    let p = dir.path();
    gg(p, &["checkout", "-q", "-b", "hotfix/1.0.1", "main"]);
    std::fs::write(p.join("a.txt"), "hotfix\n").unwrap();
    gg(p, &["commit", "-q", "-am", "hotfix"]);
    gg(p, &["checkout", "-q", "develop"]);
    std::fs::write(p.join("a.txt"), "develop\n").unwrap();
    gg(p, &["commit", "-q", "-am", "develop edit"]);
    gg(p, &["checkout", "-q", "main"]);
    let mut rt = screen(p);
    click(&mut rt, "gfk0");
    settle(&mut rt, |s| s.gitflow.selected.is_some());
    click(&mut rt, "gf-finish");
    settle(&mut rt, |s| s.gitflow.plan.is_some());
    click(&mut rt, "gf-confirm");
    settle(&mut rt, |s| s.conflicts.open);
    assert!(
        rt.state.notice.as_deref().is_some_and(
            |n| n.contains("Flux interrompu") && n.contains("l'étape restante sera reprise")
        ),
        "{:?}",
        rt.state.notice
    );
    assert_eq!(gg(p, &["tag"]), "1.0.1");
    assert!(gg(p, &["branch"]).contains("hotfix/1.0.1"));
}
