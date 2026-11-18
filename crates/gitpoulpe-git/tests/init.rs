mod common;

use std::os::unix::fs::PermissionsExt;

use common::{git as run_git, write};
use gitpoulpe_git::{Cancel, Git, InitError};

fn git() -> Git {
    Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
}

fn out(dir: &std::path::Path, args: &[&str]) -> String {
    String::from_utf8_lossy(&run_git(dir, args))
        .trim()
        .to_owned()
}

#[tokio::test]
async fn an_empty_folder_becomes_an_openable_repository_without_a_first_commit() {
    let dir = tempfile::tempdir().unwrap();
    let done = git()
        .init_repository(dir.path(), "main", &Cancel::new())
        .await
        .unwrap();
    assert!(!done.completed_existing);
    assert!(!done.info.has_commits);
    assert_eq!(
        out(dir.path(), &["symbolic-ref", "--short", "HEAD"]),
        "main"
    );
    let snapshot = git()
        .read_status(
            &done.info,
            gitpoulpe_core::Generation::default().next(),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert!(snapshot.entries.is_empty());
}

#[tokio::test]
async fn files_already_in_the_folder_show_up_as_untracked() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notes.txt", "hello\n");
    let done = git()
        .init_repository(dir.path(), "trunk", &Cancel::new())
        .await
        .unwrap();
    assert_eq!(
        out(dir.path(), &["symbolic-ref", "--short", "HEAD"]),
        "trunk"
    );
    let snapshot = git()
        .read_status(
            &done.info,
            gitpoulpe_core::Generation::default().next(),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.entries.len(), 1);
}

#[tokio::test]
async fn an_existing_repository_is_not_reinitialised() {
    let dir = common::init_repo();
    write(dir.path(), "a.txt", "x\n");
    common::commit_all(dir.path(), "first");
    let head = out(dir.path(), &["rev-parse", "HEAD"]);
    let error = git()
        .init_repository(dir.path(), "other", &Cancel::new())
        .await
        .unwrap_err();
    assert!(
        matches!(error, InitError::AlreadyRepository(_)),
        "{error:?}"
    );
    assert_eq!(out(dir.path(), &["rev-parse", "HEAD"]), head);
    assert_eq!(
        out(dir.path(), &["symbolic-ref", "--short", "HEAD"]),
        "main"
    );
}

#[tokio::test]
async fn a_bare_repository_counts_as_already_versioned() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "--bare"]);
    let error = git()
        .init_repository(dir.path(), "main", &Cancel::new())
        .await
        .unwrap_err();
    assert!(
        matches!(error, InitError::AlreadyRepository(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_subfolder_of_a_repository_is_refused_with_the_enclosing_root() {
    let repo = common::init_repo();
    let sub = repo.path().join("inner");
    std::fs::create_dir(&sub).unwrap();
    let error = git()
        .init_repository(&sub, "main", &Cancel::new())
        .await
        .unwrap_err();
    let InitError::InsideRepository(root) = error else {
        panic!("expected InsideRepository");
    };
    assert_eq!(root, std::fs::canonicalize(repo.path()).unwrap());
    assert!(!sub.join(".git").exists());
}

#[tokio::test]
async fn an_invalid_branch_name_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["", "-x", "a b", "a..b", "@", "x\ny", "bad~name", "end.lock"] {
        let error = git()
            .init_repository(dir.path(), name, &Cancel::new())
            .await
            .unwrap_err();
        assert_eq!(error, InitError::InvalidBranch, "{name:?}");
    }
    assert!(!dir.path().join(".git").exists());
}

#[tokio::test]
async fn missing_and_non_directory_paths_are_identified() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        git()
            .init_repository(&dir.path().join("nope"), "main", &Cancel::new())
            .await
            .unwrap_err(),
        InitError::NotFound
    );
    write(dir.path(), "file", "x");
    assert_eq!(
        git()
            .init_repository(&dir.path().join("file"), "main", &Cancel::new())
            .await
            .unwrap_err(),
        InitError::NotADirectory
    );
}

#[tokio::test]
async fn a_read_only_folder_is_reported_and_a_retry_succeeds_once_writable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let writable = std::fs::File::create(dir.path().join("probe")).is_ok();
    if writable {
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        return;
    }
    let error = git()
        .init_repository(dir.path(), "main", &Cancel::new())
        .await
        .unwrap_err();
    assert_eq!(error, InitError::PermissionDenied);
    assert!(!dir.path().join(".git").exists());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    git()
        .init_repository(dir.path(), "main", &Cancel::new())
        .await
        .unwrap();
}

#[tokio::test]
async fn an_interrupted_creation_is_completed_by_a_new_attempt() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
    assert!(git().open(dir.path(), &Cancel::new()).await.is_err());
    let done = git()
        .init_repository(dir.path(), "main", &Cancel::new())
        .await
        .unwrap();
    assert!(done.completed_existing);
    assert_eq!(
        out(dir.path(), &["symbolic-ref", "--short", "HEAD"]),
        "main"
    );
}

#[tokio::test]
async fn the_default_branch_follows_the_configuration_and_ignores_invalid_values() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("gitconfig");
    std::fs::write(&config, "[init]\n\tdefaultBranch = develop\n").unwrap();
    let configured = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", &config)
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null");
    assert_eq!(configured.default_branch_name(dir.path()).await, "develop");
    std::fs::write(&config, "[init]\n\tdefaultBranch = bad name\n").unwrap();
    assert_eq!(configured.default_branch_name(dir.path()).await, "main");
    assert_eq!(git().default_branch_name(dir.path()).await, "main");
}
