mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gitpoulpe_core::{Generation, ReadError, RepoId, RepoKind, SessionId, SessionInfo};
use gitpoulpe_git::{Cancel, Git};

fn session_in(dir: &Path) -> SessionInfo {
    SessionInfo {
        id: SessionId::next(),
        repo: RepoId::from_common_dir(&dir.join(".git")),
        kind: RepoKind::Ordinary,
        worktree: Some(dir.to_path_buf()),
        git_dir: dir.join(".git"),
        common_dir: dir.join(".git"),
        has_commits: true,
    }
}

fn fake_git(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("fake-git");
    let script = format!("#!/bin/sh\ncase \"$*\" in *\" config \"*) exit 1;; esac\n{body}\n");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[tokio::test]
async fn cancelling_terminates_and_reaps_the_git_process() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let program = fake_git(
        dir.path(),
        &format!("echo $$ > '{}'\nexec sleep 60", pid_file.display()),
    );
    let git = Git::with_program(program);
    let info = session_in(dir.path());
    let cancel = Cancel::new();

    let task = {
        let (git, info, cancel) = (git.clone(), info.clone(), cancel.clone());
        tokio::spawn(async move {
            git.read_status(&info, Generation::default().next(), &cancel)
                .await
        })
    };
    let pid = loop {
        if let Ok(text) = std::fs::read_to_string(&pid_file)
            && let Ok(pid) = text.trim().parse::<u32>()
        {
            break pid;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(Path::new(&format!("/proc/{pid}")).exists());

    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("cancel returns promptly")
        .unwrap();

    assert_eq!(result.unwrap_err(), ReadError::Cancelled);
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "process {pid} was left running or unreaped"
    );
}

#[tokio::test]
async fn a_failure_keeps_its_exit_code_and_a_filtered_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let program = fake_git(
        dir.path(),
        "echo \"fatal: unable to access 'https://user:s3cret@example.com/r.git/'\" >&2\nexit 3",
    );
    let git = Git::with_program(program);
    let err = git
        .read_status(
            &session_in(dir.path()),
            Generation::default().next(),
            &Cancel::new(),
        )
        .await
        .unwrap_err();
    let ReadError::GitFailed(diagnostic) = err else {
        panic!("expected GitFailed, got {err:?}")
    };
    assert_eq!(diagnostic.exit_code, Some(3));
    assert!(
        diagnostic
            .message
            .contains("https://***@example.com/r.git/")
    );
    assert!(!diagnostic.message.contains("s3cret"));
}

#[tokio::test]
async fn a_missing_git_program_is_reported_with_its_cause() {
    let dir = tempfile::tempdir().unwrap();
    let git = Git::with_program(dir.path().join("does-not-exist"));
    let err = git
        .read_status(
            &session_in(dir.path()),
            Generation::default().next(),
            &Cancel::new(),
        )
        .await
        .unwrap_err();
    let ReadError::GitUnavailable(diagnostic) = err else {
        panic!("expected GitUnavailable, got {err:?}")
    };
    assert!(
        diagnostic
            .cause
            .as_deref()
            .is_some_and(|c| c.contains("NotFound"))
    );
}

#[tokio::test]
async fn malformed_output_is_a_parse_error_and_ends_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let program = fake_git(dir.path(), "printf 'garbage\\0'\nexec sleep 60");
    let git = Git::with_program(program);
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        git.read_status(
            &session_in(dir.path()),
            Generation::default().next(),
            &Cancel::new(),
        ),
    )
    .await
    .expect("a parse error stops the process instead of waiting for it");
    assert!(matches!(result.unwrap_err(), ReadError::Parse(_)));
}

#[tokio::test]
async fn a_technical_failure_never_looks_like_a_clean_repository() {
    let dir = tempfile::tempdir().unwrap();
    let program = fake_git(dir.path(), "exit 128");
    let git = Git::with_program(program);
    let result = git
        .read_status(
            &session_in(dir.path()),
            Generation::default().next(),
            &Cancel::new(),
        )
        .await;
    assert!(matches!(result, Err(ReadError::GitFailed(_))));
}

#[tokio::test]
async fn a_program_busy_for_writing_is_retried_instead_of_reported_missing() {
    let dir = tempfile::tempdir().unwrap();
    let program = fake_git(dir.path(), "exit 3");
    let writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&program)
        .unwrap();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        drop(writer);
    });
    let git = Git::with_program(program);
    let err = git
        .read_status(
            &session_in(dir.path()),
            Generation::default().next(),
            &Cancel::new(),
        )
        .await
        .unwrap_err();
    let ReadError::GitFailed(diagnostic) = err else {
        panic!("expected GitFailed, got {err:?}")
    };
    assert_eq!(diagnostic.exit_code, Some(3));
}
