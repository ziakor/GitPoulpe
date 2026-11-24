use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use gitpoulpe_git::Git;
use rune::Runtime;

use super::{click_at, rect_of, runtime, step};
use crate::state::State;

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
    panic!("the screen never settled");
}

fn click(rt: &mut Runtime<State>, key: &str) {
    if key.starts_with("lfs-") && !key.starts_with("lfs-row") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("lfs-panel", target));
        for _ in 0..8 {
            step(rt);
        }
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(
        p,
        &["remote", "add", "origin", "https://127.0.0.1:9/none.git"],
    );
    std::fs::write(
        p.join(".gitattributes"),
        "*.bin filter=lfs diff=lfs merge=lfs -text\n",
    )
    .unwrap();
    let oid = "b".repeat(64);
    std::fs::write(
        p.join("big.bin"),
        format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize 1048576\n"),
    )
    .unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "lfs pointer"]);
    dir
}

fn git_only_path() -> (tempfile::TempDir, std::ffi::OsString) {
    let dir = tempfile::tempdir().unwrap();
    let real = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|d| d.join("git"))
        .find(|c| c.is_file())
        .unwrap();
    std::os::unix::fs::symlink(real, dir.path().join("git")).unwrap();
    let path = dir.path().as_os_str().to_owned();
    (dir, path)
}

fn screen(dir: &Path, engine_git: Git) -> Runtime<State> {
    let mut state = State::new(None, rune::prefs::Prefs::memory());
    state.engine_git = Some(engine_git);
    state.setup(Arc::new(|| {}));
    state.open_blocking(dir.to_path_buf(), Duration::from_secs(20));
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    super::menu(&mut rt, "nav-lfs");
    settle(&mut rt, |s| s.lfs.open && !s.lfs.loading);
    rt
}

fn base_git(path: &std::ffi::OsStr) -> Git {
    Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("PATH", path)
}

#[test]
fn without_git_lfs_the_pointers_are_listed_and_the_way_to_enable_the_support_is_explained() {
    let dir = repo();
    let (_keep, path) = git_only_path();
    let rt = screen(dir.path(), base_git(&path));
    assert!(shows(&rt, "Git LFS n'est pas installé"));
    assert!(shows(&rt, "n'installe ni ne télécharge rien"));
    assert!(shows(&rt, "git lfs install"));
    assert!(shows(&rt, "objet LFS absent"));
    assert!(shows(&rt, "big.bin"));
    assert!(shows(
        &rt,
        "aucun objet ne peut être transféré sans l'outil"
    ));
    assert!(rect_of(&rt, "lfs-pull").is_none() && rect_of(&rt, "lfs-fetch").is_none());
}

fn tool_path(name: &str) -> String {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|d| d.join(name))
        .find(|c| c.is_file())
        .unwrap_or_else(|| panic!("{name} is on the PATH"))
        .display()
        .to_string()
}

fn fake_tool(body: &str) -> (tempfile::TempDir, std::ffi::OsString) {
    let dir = tempfile::tempdir().unwrap();
    let real = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|d| d.join("git"))
        .find(|c| c.is_file())
        .unwrap();
    std::os::unix::fs::symlink(real, dir.path().join("git")).unwrap();
    let script = dir.path().join("git-lfs");
    std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = dir.path().as_os_str().to_owned();
    (dir, path)
}

#[test]
fn with_the_tool_a_transfer_reports_its_progress_and_finishes() {
    let dir = repo();
    let (_keep, path) = fake_tool(
        r#"case "$1" in
  version) echo "git-lfs/3.5.1" ;;
  *) echo "Downloading LFS objects: 100% (1/1), 1.0 MB | 1 MB/s, done." >&2 ;;
esac"#,
    );
    let mut rt = screen(dir.path(), base_git(&path));
    assert!(shows(&rt, "Git LFS installé : git-lfs/3.5.1"));
    click(&mut rt, "lfs-pull");
    settle(&mut rt, |s| s.lfs.running.is_none() && s.lfs.done.is_some());
    assert!(shows(&rt, "Téléchargement des objets LFS terminée."));
}

#[test]
fn a_running_transfer_can_be_cancelled_and_a_server_error_is_explained() {
    let dir = repo();
    let (keep, path) = fake_tool(&format!(
        r#"case "$1" in
  version) echo "git-lfs/3.5.1" ;;
  fetch) {1} 60 & echo $! > "{0}/sleeper"; wait ;;
  *) echo "Post https://host/objects/batch: 502 Bad Gateway" >&2; exit 2 ;;
esac"#,
        std::env::temp_dir().display(),
        tool_path("sleep")
    ));
    let _ = keep.path();
    let mut rt = screen(dir.path(), base_git(&path));
    click(&mut rt, "lfs-fetch");
    for _ in 0..40 {
        step(&mut rt);
        if rt.state.lfs.running.is_some() {
            break;
        }
    }
    assert!(rt.state.lfs.running.is_some());
    click(&mut rt, "lfs-cancel");
    settle(&mut rt, |s| {
        s.lfs.running.is_none() && s.lfs.error.is_some()
    });
    assert!(shows(&rt, "opération annulée"));
    click(&mut rt, "lfs-push");
    settle(&mut rt, |s| {
        s.lfs.running.is_none()
            && s.lfs
                .error
                .as_deref()
                .is_some_and(|e| e.contains("serveur LFS"))
    });
    assert!(shows(&rt, "erreur du serveur LFS"));
}
