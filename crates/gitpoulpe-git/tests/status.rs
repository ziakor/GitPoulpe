mod common;

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use common::*;
use gitpoulpe_core::{
    EntryKind, Generation, OpenError, ReadError, RepoKind, StatusCode, StatusSnapshot,
};
use gitpoulpe_git::{Cancel, Git};

async fn open_and_read(path: &Path) -> (gitpoulpe_core::SessionInfo, StatusSnapshot) {
    let git = Git::new();
    let cancel = Cancel::new();
    let info = git.open(path, &cancel).await.unwrap();
    let snapshot = git
        .read_status(&info, Generation::default().next(), &cancel)
        .await
        .unwrap();
    (info, snapshot)
}

fn paths(
    entries: impl Iterator<Item = impl std::borrow::Borrow<gitpoulpe_core::ChangeEntry>>,
) -> Vec<PathBuf> {
    sorted(entries.map(|e| e.borrow().path.clone()).collect())
}

#[tokio::test]
async fn states_match_git_for_every_kind_of_change() {
    let repo = init_repo();
    let dir = repo.path();
    for name in [
        "modified.txt",
        "deleted.txt",
        "renamed.txt",
        "staged-modified.txt",
    ] {
        write(dir, name, &format!("{name}\nline two\nline three\n"));
    }
    commit_all(dir, "base");
    write(dir, "modified.txt", "changed\n");
    std::fs::remove_file(dir.join("deleted.txt")).unwrap();
    git(dir, &["mv", "renamed.txt", "moved.txt"]);
    write(dir, "staged-modified.txt", "staged change\n");
    git(dir, &["add", "staged-modified.txt"]);
    write(dir, "added.txt", "new\n");
    git(dir, &["add", "added.txt"]);
    write(dir, "untracked.txt", "u\n");

    let (info, snapshot) = open_and_read(dir).await;

    assert_eq!(info.kind, RepoKind::Ordinary);
    assert!(info.has_commits);
    assert_eq!(snapshot.branch.head.as_deref(), Some("main"));

    let git_unstaged = sorted(
        [
            nul_paths(&git(dir, &["diff", "--name-only", "-z"])),
            nul_paths(&git(
                dir,
                &["ls-files", "--others", "--exclude-standard", "-z"],
            )),
        ]
        .concat(),
    );
    assert_eq!(paths(snapshot.unstaged()), git_unstaged);
    assert_eq!(
        paths(snapshot.staged()),
        sorted(nul_paths(&git(
            dir,
            &["diff", "--cached", "--name-only", "-z"]
        )))
    );

    let renamed = snapshot
        .entries
        .iter()
        .find(|e| e.kind == EntryKind::Renamed)
        .unwrap();
    assert_eq!(renamed.path, Path::new("moved.txt"));
    assert_eq!(
        renamed.original_path.as_deref(),
        Some(Path::new("renamed.txt"))
    );
    let deleted = snapshot
        .entries
        .iter()
        .find(|e| e.path == Path::new("deleted.txt"))
        .unwrap();
    assert_eq!(
        (deleted.index, deleted.worktree),
        (StatusCode::Unmodified, StatusCode::Deleted)
    );
    let both = snapshot
        .entries
        .iter()
        .find(|e| e.path == Path::new("staged-modified.txt"))
        .unwrap();
    assert!(both.is_staged() && !both.is_unstaged());
}

#[tokio::test]
async fn unusual_paths_are_preserved_byte_for_byte() {
    let repo = init_repo();
    let dir = repo.path();
    let names: Vec<OsString> = vec![
        "with space.txt".into(),
        "tab\there.txt".into(),
        "new\nline.txt".into(),
        "é accentué 日本語.txt".into(),
        "-leading-dash.txt".into(),
        OsString::from_vec(b"bad\xff\xfename.txt".to_vec()),
    ];
    for name in &names {
        std::fs::write(dir.join(name), "x").unwrap();
    }
    let (_, snapshot) = open_and_read(dir).await;
    let expected = sorted(nul_paths(&git(dir, &["ls-files", "--others", "-z"])));
    assert_eq!(expected.len(), names.len());
    assert_eq!(paths(snapshot.unstaged()), expected);
    for name in &names {
        assert!(
            snapshot
                .entries
                .iter()
                .any(|e| e.path.as_os_str() == name.as_os_str()),
            "{name:?} missing"
        );
    }
}

#[tokio::test]
async fn a_repository_without_a_first_commit_uses_an_empty_base() {
    let repo = init_repo();
    write(repo.path(), "a.txt", "a\n");
    git(repo.path(), &["add", "a.txt"]);
    write(repo.path(), "b.txt", "b\n");
    let (info, snapshot) = open_and_read(repo.path()).await;
    assert!(!info.has_commits);
    let a = snapshot
        .entries
        .iter()
        .find(|e| e.path == Path::new("a.txt"))
        .unwrap();
    assert_eq!(a.index, StatusCode::Added);
    assert_eq!(paths(snapshot.unstaged()), vec![PathBuf::from("b.txt")]);
}

#[tokio::test]
async fn a_clean_repository_has_no_entries() {
    let repo = init_repo();
    write(repo.path(), "a.txt", "a\n");
    commit_all(repo.path(), "base");
    let (_, snapshot) = open_and_read(repo.path()).await;
    assert!(snapshot.entries.is_empty());
}

#[tokio::test]
async fn merge_conflicts_are_unmerged_entries() {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "c.txt", "base\n");
    commit_all(dir, "base");
    git(dir, &["checkout", "-q", "-b", "other"]);
    write(dir, "c.txt", "other\n");
    commit_all(dir, "other");
    git(dir, &["checkout", "-q", "main"]);
    write(dir, "c.txt", "main\n");
    commit_all(dir, "main");
    let merge = git_output(dir, &["merge", "other"]);
    assert!(!merge.status.success());

    let (_, snapshot) = open_and_read(dir).await;
    let conflicts: Vec<_> = snapshot.conflicts().collect();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].path, Path::new("c.txt"));
    assert_eq!(conflicts[0].kind, EntryKind::Unmerged);
    assert!(!conflicts[0].is_staged() && !conflicts[0].is_unstaged());
}

#[tokio::test]
async fn a_linked_worktree_is_its_own_session_of_the_same_repository() {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "a.txt", "a\n");
    commit_all(dir, "base");
    let holder = tempfile::tempdir().unwrap();
    let linked = holder.path().join("linked");
    git(
        dir,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            linked.to_str().unwrap(),
        ],
    );
    write(&linked, "only-in-linked.txt", "x\n");

    let (main_info, main_status) = open_and_read(dir).await;
    let (linked_info, linked_status) = open_and_read(&linked).await;

    assert_eq!(linked_info.kind, RepoKind::LinkedWorktree);
    assert_eq!(main_info.repo, linked_info.repo);
    assert_ne!(main_info.id, linked_info.id);
    assert_eq!(
        linked_info
            .worktree
            .as_deref()
            .map(|p| p.canonicalize().unwrap()),
        Some(linked.canonicalize().unwrap())
    );
    assert!(main_status.entries.is_empty());
    assert_eq!(
        paths(linked_status.unstaged()),
        vec![PathBuf::from("only-in-linked.txt")]
    );
    assert_eq!(linked_status.branch.head.as_deref(), Some("side"));
}

#[tokio::test]
async fn a_bare_repository_opens_without_a_worktree_and_status_says_so() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "--bare", "-b", "main"]);
    let git_cli = Git::new();
    let cancel = Cancel::new();
    let info = git_cli.open(dir.path(), &cancel).await.unwrap();
    assert_eq!(info.kind, RepoKind::Bare);
    assert!(info.worktree.is_none());
    assert!(!info.has_commits);
    let err = git_cli
        .read_status(&info, Generation::default().next(), &cancel)
        .await
        .unwrap_err();
    assert_eq!(err, ReadError::NoWorktree);
}

#[tokio::test]
async fn opening_a_subdirectory_reports_the_repository_root() {
    let repo = init_repo();
    write(repo.path(), "deep/er/file.txt", "x\n");
    let (info, snapshot) = open_and_read(&repo.path().join("deep/er")).await;
    assert_eq!(
        info.worktree.as_deref().map(|p| p.canonicalize().unwrap()),
        Some(repo.path().canonicalize().unwrap())
    );
    assert_eq!(
        paths(snapshot.unstaged()),
        vec![PathBuf::from("deep/er/file.txt")]
    );
}

#[tokio::test]
async fn open_errors_are_explicit_states() {
    let git_cli = Git::new();
    let cancel = Cancel::new();
    let dir = tempfile::tempdir().unwrap();

    assert_eq!(
        git_cli
            .open(&dir.path().join("missing"), &cancel)
            .await
            .unwrap_err(),
        OpenError::NotFound
    );

    let file = dir.path().join("file.txt");
    std::fs::write(&file, "x").unwrap();
    assert_eq!(
        git_cli.open(&file, &cancel).await.unwrap_err(),
        OpenError::NotADirectory
    );

    assert_eq!(
        git_cli.open(dir.path(), &cancel).await.unwrap_err(),
        OpenError::NotARepository
    );

    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::create_dir(locked.join("inside")).unwrap();
    std::fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    let denied = git_cli.open(&locked.join("inside"), &cancel).await;
    std::fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    assert_eq!(denied.unwrap_err(), OpenError::PermissionDenied);

    let cancelled = Cancel::new();
    cancelled.cancel();
    assert_eq!(
        git_cli.open(dir.path(), &cancelled).await.unwrap_err(),
        OpenError::Cancelled
    );
}

#[tokio::test]
async fn inspecting_never_changes_the_repository() {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "a.txt", "a\n");
    write(dir, "b.txt", "b\n");
    commit_all(dir, "base");
    write(dir, "a.txt", "changed\n");
    write(dir, "new.txt", "n\n");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let before = tree_fingerprint(dir);

    let git_cli = Git::new();
    let cancel = Cancel::new();
    let info = git_cli.open(dir, &cancel).await.unwrap();
    for generation in 1..=3u32 {
        let mut g = Generation::default();
        for _ in 0..generation {
            g = g.next();
        }
        git_cli.read_status(&info, g, &cancel).await.unwrap();
    }
    let cancelled = Cancel::new();
    cancelled.cancel();
    assert_eq!(
        git_cli
            .read_status(&info, Generation::default().next(), &cancelled)
            .await
            .unwrap_err(),
        ReadError::Cancelled
    );

    assert_eq!(tree_fingerprint(dir), before);
}

#[tokio::test]
async fn a_repository_configured_fsmonitor_program_is_never_run() {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "a.txt", "a\n");
    commit_all(dir, "base");
    let marker = tempfile::tempdir().unwrap();
    let marker_file = marker.path().join("ran");
    let hook = marker.path().join("hook.sh");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker_file.display()),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    git(dir, &["config", "core.fsmonitor", hook.to_str().unwrap()]);
    write(dir, "b.txt", "b\n");

    open_and_read(dir).await;

    assert!(
        !marker_file.exists(),
        "the configured fsmonitor program was executed"
    );
}

#[tokio::test]
async fn a_large_status_is_streamed_and_complete() {
    let repo = init_repo();
    for i in 0..4000 {
        write(repo.path(), &format!("dir{}/file-{i:05}.txt", i % 40), "x");
    }
    let (_, snapshot) = open_and_read(repo.path()).await;
    assert_eq!(snapshot.unstaged().count(), 4000);
}

#[tokio::test]
async fn each_snapshot_carries_its_repository_session_and_generation() {
    let repo = init_repo();
    let git_cli = Git::new();
    let cancel = Cancel::new();
    let info = git_cli.open(repo.path(), &cancel).await.unwrap();
    let first = Generation::default().next();
    let second = first.next();
    let (a, b) = tokio::join!(
        git_cli.read_status(&info, first, &cancel),
        git_cli.read_status(&info, second, &cancel)
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!((a.generation, b.generation), (first, second));
    assert!(
        a.repo == info.repo && b.repo == info.repo && a.session == info.id && b.session == info.id
    );
}
