mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{git as run_git, init_repo, write};
use gitpoulpe_core::{
    CommitFailureKind, CommitOutcome, CommitRequest, Generation, Profile, SessionInfo,
    SigningPreference, StatusSnapshot,
};
use gitpoulpe_git::{Cancel, Git, HookError};

fn git_with(config: Option<&Path>) -> Git {
    let git = Git::new()
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com");
    match config {
        Some(file) => git.with_env("GIT_CONFIG_GLOBAL", file.as_os_str()),
        None => git.with_env("GIT_CONFIG_GLOBAL", "/dev/null"),
    }
}

fn write_hook(dir: &Path, name: &str, body: &str, mode: u32) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    path
}

async fn open(git: &Git, dir: &Path) -> SessionInfo {
    git.open(dir, &Cancel::new()).await.unwrap()
}

fn profile() -> Profile {
    Profile {
        id: "p".into(),
        label: "P".into(),
        user_name: "Ada Lovelace".into(),
        user_email: "ada@example.org".into(),
        signing: SigningPreference::None,
    }
}

async fn staged(git: &Git, info: &SessionInfo) -> (StatusSnapshot, CommitRequest) {
    let snapshot = git
        .read_status(info, Generation::default().next(), &Cancel::new())
        .await
        .unwrap();
    let request = CommitRequest {
        message: "msg".to_owned(),
        staged: snapshot
            .entries
            .iter()
            .filter(|e| e.is_staged())
            .map(|e| {
                (
                    e.path.clone(),
                    snapshot.index_ids.get(&e.path).cloned().unwrap_or_default(),
                )
            })
            .collect(),
        profile: Some(profile()),
    };
    (snapshot, request)
}

#[tokio::test]
async fn the_default_hooks_directory_lists_active_disabled_and_foreign_files() {
    let repo = init_repo();
    let hooks = repo.path().join(".git/hooks");
    write_hook(&hooks, "pre-commit", "exit 0", 0o755);
    write_hook(&hooks, "commit-msg", "exit 0", 0o644);
    write_hook(&hooks, "helper.sh", "exit 0", 0o755);
    write_hook(&hooks, "pre-push.sample", "exit 0", 0o755);
    let git = git_with(None);
    let info = open(&git, repo.path()).await;
    let view = git.list_hooks(&info).await.unwrap();
    assert_eq!(
        view.dir,
        std::fs::canonicalize(&hooks)
            .unwrap()
            .parent()
            .unwrap()
            .join("hooks")
    );
    assert_eq!(view.configured_path, None);
    assert!(!view.shared && !view.in_worktree);
    let names: Vec<(&str, bool, bool)> = view
        .hooks
        .iter()
        .map(|h| (h.name.as_str(), h.known, h.active()))
        .collect();
    assert_eq!(
        names,
        [
            ("commit-msg", true, false),
            ("pre-commit", true, true),
            ("helper.sh", false, false)
        ]
    );
    assert_eq!(view.hooks[1].interpreter.as_deref(), Some("/bin/sh"));
}

#[tokio::test]
async fn a_custom_hooks_path_is_reported_with_its_scope_and_where_it_lives() {
    let repo = init_repo();
    write_hook(
        &repo.path().join(".githooks"),
        "pre-commit",
        "exit 0",
        0o755,
    );
    run_git(repo.path(), &["config", "core.hooksPath", ".githooks"]);
    let git = git_with(None);
    let info = open(&git, repo.path()).await;
    let view = git.list_hooks(&info).await.unwrap();
    assert_eq!(
        view.dir,
        std::fs::canonicalize(repo.path().join(".githooks")).unwrap()
    );
    assert_eq!(view.configured_path.as_deref(), Some(".githooks"));
    assert_eq!(view.configured_scope.as_deref(), Some("local"));
    assert!(view.in_worktree && view.shared);
    assert_eq!(view.hooks.len(), 1);
    let outside = tempfile::tempdir().unwrap();
    write_hook(outside.path(), "pre-push", "exit 0", 0o755);
    let config = outside.path().join("gitconfig");
    std::fs::write(
        &config,
        format!("[core]\n\thooksPath = {}\n", outside.path().display()),
    )
    .unwrap();
    let plain = init_repo();
    let git = git_with(Some(&config));
    let info = open(&git, plain.path()).await;
    let view = git.list_hooks(&info).await.unwrap();
    assert_eq!(view.configured_scope.as_deref(), Some("global"));
    assert!(view.shared && !view.in_worktree);
    assert_eq!(view.hooks[0].name, "pre-push");
}

#[tokio::test]
async fn disabling_a_hook_really_stops_git_from_running_it_and_enabling_brings_it_back() {
    let repo = init_repo();
    write(repo.path(), "a.txt", "a\n");
    run_git(repo.path(), &["add", "a.txt"]);
    write_hook(
        &repo.path().join(".git/hooks"),
        "pre-commit",
        "echo refused >&2\nexit 1",
        0o755,
    );
    let git = git_with(None);
    let info = open(&git, repo.path()).await;
    let (_, request) = staged(&git, &info).await;
    let outcome = git.commit(&info, &request, &Cancel::new()).await.unwrap();
    assert!(
        matches!(
            outcome,
            CommitOutcome::Failed {
                kind: CommitFailureKind::Hook,
                ..
            }
        ),
        "{outcome:?}"
    );
    let view = git
        .set_hook_enabled(&info, "pre-commit", false, false)
        .await
        .unwrap();
    assert!(!view.hooks[0].active());
    let (_, request) = staged(&git, &info).await;
    let outcome = git.commit(&info, &request, &Cancel::new()).await.unwrap();
    assert!(
        matches!(outcome, CommitOutcome::Created { .. }),
        "{outcome:?}"
    );
    write(repo.path(), "b.txt", "b\n");
    run_git(repo.path(), &["add", "b.txt"]);
    let view = git
        .set_hook_enabled(&info, "pre-commit", true, false)
        .await
        .unwrap();
    assert!(view.hooks[0].active());
    let (_, request) = staged(&git, &info).await;
    let outcome = git.commit(&info, &request, &Cancel::new()).await.unwrap();
    assert!(
        matches!(
            outcome,
            CommitOutcome::Failed {
                kind: CommitFailureKind::Hook,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(
        git.set_hook_enabled(&info, "pre-commit", true, false)
            .await
            .unwrap_err(),
        HookError::AlreadyInState
    );
}

#[tokio::test]
async fn a_hook_shared_outside_the_repository_changes_only_after_an_explicit_confirmation() {
    let shared = tempfile::tempdir().unwrap();
    let hook = write_hook(shared.path(), "pre-commit", "exit 0", 0o755);
    let config = shared.path().join("gitconfig");
    std::fs::write(
        &config,
        format!("[core]\n\thooksPath = {}\n", shared.path().display()),
    )
    .unwrap();
    let repo = init_repo();
    let git = git_with(Some(&config));
    let info = open(&git, repo.path()).await;
    let error = git
        .set_hook_enabled(&info, "pre-commit", false, false)
        .await
        .unwrap_err();
    assert!(
        matches!(error, HookError::SharedNeedsConfirmation(_)),
        "{error:?}"
    );
    assert_eq!(
        std::fs::metadata(&hook).unwrap().permissions().mode() & 0o111,
        0o111
    );
    git.set_hook_enabled(&info, "pre-commit", false, true)
        .await
        .unwrap();
    assert_eq!(
        std::fs::metadata(&hook).unwrap().permissions().mode() & 0o111,
        0
    );
}

#[tokio::test]
async fn unknown_names_traversal_symlinks_and_missing_hooks_are_refused() {
    let repo = init_repo();
    let hooks = repo.path().join(".git/hooks");
    let target = tempfile::tempdir().unwrap();
    let real = write_hook(target.path(), "real", "exit 0", 0o755);
    std::fs::create_dir_all(&hooks).unwrap();
    std::os::unix::fs::symlink(&real, hooks.join("pre-commit")).unwrap();
    let git = git_with(None);
    let info = open(&git, repo.path()).await;
    for name in ["../config", "helper.sh", "", "pre-commit/../x"] {
        assert_eq!(
            git.set_hook_enabled(&info, name, false, true)
                .await
                .unwrap_err(),
            HookError::UnknownHook,
            "{name:?}"
        );
    }
    assert_eq!(
        git.set_hook_enabled(&info, "pre-push", false, true)
            .await
            .unwrap_err(),
        HookError::NoSuchHook
    );
    assert_eq!(
        git.set_hook_enabled(&info, "pre-commit", false, true)
            .await
            .unwrap_err(),
        HookError::Symlink
    );
    assert_eq!(
        std::fs::metadata(&real).unwrap().permissions().mode() & 0o111,
        0o111
    );
}

#[tokio::test]
async fn a_failing_hook_keeps_its_cause_but_its_secret_output_is_filtered_and_nothing_is_bypassed()
{
    let repo = init_repo();
    write(repo.path(), "a.txt", "a\n");
    run_git(repo.path(), &["add", "a.txt"]);
    write_hook(
        &repo.path().join(".git/hooks"),
        "pre-commit",
        "echo 'lint failed: a.txt has trailing spaces' >&2\necho 'API_TOKEN=s3cretvalue123 password: hunter2' >&2\necho 'Authorization: Bearer abcdef.ghi' >&2\necho 'key ghp_abcdefghijklmnopqrstuvwxyz0123' >&2\nexit 3",
        0o755,
    );
    let git = git_with(None);
    let info = open(&git, repo.path()).await;
    let (_, request) = staged(&git, &info).await;
    let outcome = git.commit(&info, &request, &Cancel::new()).await.unwrap();
    let CommitOutcome::Failed { kind, output } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(kind, CommitFailureKind::Hook);
    assert!(
        output.contains("lint failed: a.txt has trailing spaces"),
        "{output}"
    );
    for leaked in [
        "s3cretvalue123",
        "hunter2",
        "abcdef.ghi",
        "ghp_abcdefghijklmnop",
    ] {
        assert!(!output.contains(leaked), "{leaked} leaked: {output}");
    }
    assert!(output.contains("***"));
    assert!(
        common::git_output(repo.path(), &["rev-parse", "--verify", "-q", "HEAD"])
            .stdout
            .is_empty()
    );
    assert!(run_git(repo.path(), &["diff", "--cached", "--name-only"]).starts_with(b"a.txt"));
}
