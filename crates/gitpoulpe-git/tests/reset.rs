mod common;

use std::path::{Path, PathBuf};

use common::{commit_all, git as run_git, git_output, init_repo, write};
use gitpoulpe_core::{ResetMode, SessionInfo};
use gitpoulpe_git::{Cancel, Git, ResetError};

struct Env {
    repo: tempfile::TempDir,
    git: Git,
    info: SessionInfo,
    c1: String,
    c3: String,
}

impl Env {
    fn path(&self) -> &Path {
        self.repo.path()
    }
    fn out(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&run_git(self.path(), args))
            .trim()
            .to_owned()
    }
    fn snapshot(&self) -> (String, String, String, String) {
        (
            self.out(&["rev-parse", "HEAD"]),
            self.out(&["ls-files", "--stage"]),
            std::fs::read_to_string(self.path().join("a.txt")).unwrap(),
            std::fs::read_to_string(self.path().join("b.txt")).unwrap(),
        )
    }
}

async fn env() -> Env {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "a1\n");
    write(p, "b.txt", "b1\n");
    commit_all(p, "c1");
    write(p, "a.txt", "a2\n");
    commit_all(p, "c2");
    write(p, "b.txt", "b3\n");
    commit_all(p, "c3");
    let c1 = String::from_utf8_lossy(&run_git(p, &["rev-parse", "HEAD~2"]))
        .trim()
        .to_owned();
    let c3 = String::from_utf8_lossy(&run_git(p, &["rev-parse", "HEAD"]))
        .trim()
        .to_owned();
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com");
    let info = git.open(p, &Cancel::new()).await.unwrap();
    Env {
        repo,
        git,
        info,
        c1,
        c3,
    }
}

fn dirty(e: &Env) {
    write(e.path(), "a.txt", "a staged edit\n");
    run_git(e.path(), &["add", "a.txt"]);
    write(e.path(), "b.txt", "b unstaged edit\n");
    write(e.path(), "u.txt", "untracked\n");
}

#[tokio::test]
async fn the_preview_lists_leaving_commits_and_what_each_mode_would_touch() {
    let e = env().await;
    dirty(&e);
    let soft = e
        .git
        .reset_preview(&e.info, &e.c1, ResetMode::Soft)
        .await
        .unwrap();
    assert_eq!(soft.branch.as_deref(), Some("main"));
    assert_eq!(
        soft.leaving
            .iter()
            .map(|c| c.subject.as_str())
            .collect::<Vec<_>>(),
        ["c3", "c2"]
    );
    assert_eq!(soft.entering, 0);
    assert_eq!(soft.staged, [PathBuf::from("a.txt")]);
    assert_eq!(soft.unstaged, [PathBuf::from("b.txt")]);
    assert!(soft.would_lose.is_empty());
    let hard = e
        .git
        .reset_preview(&e.info, &e.c1, ResetMode::Hard)
        .await
        .unwrap();
    assert_eq!(
        hard.would_lose,
        [PathBuf::from("a.txt"), PathBuf::from("b.txt")]
    );
    let forward = e
        .git
        .reset_preview(&e.info, &e.c3, ResetMode::Hard)
        .await
        .unwrap();
    assert!(forward.leaving.is_empty());
}

#[tokio::test]
async fn soft_moves_only_the_branch() {
    let e = env().await;
    dirty(&e);
    let (_, index_before, a, b) = e.snapshot();
    let outcome = e
        .git
        .reset_branch(&e.info, &e.c1, ResetMode::Soft, &e.c3, false)
        .await
        .unwrap();
    let (head, index, a_after, b_after) = e.snapshot();
    assert_eq!(head, e.c1);
    assert_eq!((index, a_after, b_after), (index_before, a, b));
    assert_eq!(outcome.old_head, e.c3);
    assert!(outcome.work_marker.is_none());
    assert!(e.path().join("u.txt").exists());
}

#[tokio::test]
async fn mixed_moves_the_branch_and_the_index_and_keeps_the_files() {
    let e = env().await;
    dirty(&e);
    let (_, _, a, b) = e.snapshot();
    e.git
        .reset_branch(&e.info, &e.c1, ResetMode::Mixed, &e.c3, false)
        .await
        .unwrap();
    let (head, _, a_after, b_after) = e.snapshot();
    assert_eq!(head, e.c1);
    assert!(e.out(&["diff", "--cached", "--name-only"]).is_empty());
    assert_eq!((a_after, b_after), (a, b));
    assert!(e.out(&["status", "--porcelain"]).contains("M a.txt"));
}

#[tokio::test]
async fn hard_needs_a_confirmation_when_it_would_erase_work_then_keeps_a_snapshot_of_it() {
    let e = env().await;
    dirty(&e);
    let before = e.snapshot();
    let refused = e
        .git
        .reset_branch(&e.info, &e.c1, ResetMode::Hard, &e.c3, false)
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        ResetError::NeedsConfirmation(vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")])
    );
    assert_eq!(e.snapshot(), before);
    let outcome = e
        .git
        .reset_branch(&e.info, &e.c1, ResetMode::Hard, &e.c3, true)
        .await
        .unwrap();
    let (head, _, a, b) = e.snapshot();
    assert_eq!(head, e.c1);
    assert_eq!((a.as_str(), b.as_str()), ("a1\n", "b1\n"));
    assert_eq!(
        std::fs::read_to_string(e.path().join("u.txt")).unwrap(),
        "untracked\n"
    );
    let work = outcome
        .work_marker
        .expect("the unsaved tracked work was snapshotted");
    assert_eq!(e.out(&["show", &format!("{work}:a.txt")]), "a staged edit");
    assert_eq!(
        e.out(&["show", &format!("{work}:b.txt")]),
        "b unstaged edit"
    );
    assert_eq!(e.out(&["rev-parse", &outcome.head_marker]), e.c3);
}

#[tokio::test]
async fn a_hard_reset_of_a_clean_tree_only_loses_commits_and_they_stay_recoverable() {
    let e = env().await;
    let outcome = e
        .git
        .reset_branch(&e.info, &e.c1, ResetMode::Hard, &e.c3, false)
        .await
        .unwrap();
    assert!(outcome.work_marker.is_none());
    assert_eq!(e.out(&["rev-parse", "HEAD"]), e.c1);
    assert_eq!(e.out(&["rev-parse", &outcome.head_marker]), e.c3);
    assert_eq!(e.out(&["rev-parse", "main@{1}"]), e.c3);
    run_git(e.path(), &["reset", "-q", "--hard", &e.c3]);
    assert_eq!(e.out(&["rev-parse", "HEAD"]), e.c3);
}

#[tokio::test]
async fn the_head_marker_keeps_the_old_tip_alive_even_after_the_reflog_is_gone() {
    let e = env().await;
    let outcome = e
        .git
        .reset_branch(&e.info, &e.c1, ResetMode::Hard, &e.c3, false)
        .await
        .unwrap();
    run_git(
        e.path(),
        &[
            "reflog",
            "expire",
            "--expire=now",
            "--expire-unreachable=now",
            "--all",
        ],
    );
    run_git(e.path(), &["gc", "-q", "--prune=now"]);
    assert_eq!(e.out(&["cat-file", "-t", &e.c3]), "commit");
    assert_eq!(e.out(&["rev-parse", &outcome.head_marker]), e.c3);
}

#[tokio::test]
async fn bad_requests_change_nothing() {
    let e = env().await;
    dirty(&e);
    let before = e.snapshot();
    let short = e.c1[..10].to_owned();
    for target in [
        "HEAD",
        "main",
        "--hard",
        "abc",
        short.as_str(),
        &"0".repeat(40),
    ] {
        assert_eq!(
            e.git
                .reset_branch(&e.info, target, ResetMode::Hard, &e.c3, true)
                .await
                .unwrap_err(),
            ResetError::InvalidTarget,
            "{target}"
        );
    }
    assert_eq!(
        e.git
            .reset_branch(&e.info, &e.c1, ResetMode::Soft, &e.c1, false)
            .await
            .unwrap_err(),
        ResetError::HeadMoved
    );
    assert_eq!(e.snapshot(), before);
    assert!(e.out(&["for-each-ref", "refs/gitpoulpe"]).is_empty());
}

#[tokio::test]
async fn no_reset_runs_during_another_operation() {
    let e = env().await;
    run_git(e.path(), &["checkout", "-q", "-b", "side", &e.c1]);
    write(e.path(), "a.txt", "side\n");
    commit_all(e.path(), "side");
    run_git(e.path(), &["checkout", "-q", "main"]);
    assert!(!git_output(e.path(), &["merge", "side"]).status.success());
    let head = e.out(&["rev-parse", "HEAD"]);
    assert_eq!(
        e.git
            .reset_branch(&e.info, &e.c1, ResetMode::Hard, &head, true)
            .await
            .unwrap_err(),
        ResetError::OperationInProgress
    );
}
