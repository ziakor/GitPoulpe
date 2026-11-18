mod common;

use std::path::Path;

use common::{commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{BranchError, Cancel, Git, ReflogError};

struct Env {
    repo: tempfile::TempDir,
    git: Git,
    info: SessionInfo,
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
}

async fn env() -> Env {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "1\n");
    commit_all(p, "one");
    write(p, "a.txt", "2\n");
    commit_all(p, "two");
    write(p, "a.txt", "3\n");
    commit_all(p, "three");
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null");
    let info = git.open(p, &Cancel::new()).await.unwrap();
    Env { repo, git, info }
}

#[tokio::test]
async fn the_entries_show_the_operation_the_commit_and_the_date_newest_first() {
    let e = env().await;
    let three = e.out(&["rev-parse", "HEAD"]);
    let two = e.out(&["rev-parse", "HEAD~1"]);
    run_git(e.path(), &["reset", "-q", "--hard", "HEAD~1"]);
    run_git(e.path(), &["checkout", "-q", "-b", "other"]);
    let entries = e.git.list_reflog(&e.info, "HEAD").await.unwrap();
    let actions: Vec<&str> = entries.iter().map(|x| x.action.as_str()).collect();
    assert_eq!(
        actions,
        ["checkout", "reset", "commit", "commit", "commit (initial)"]
    );
    assert_eq!(entries[0].selector, "HEAD@{0}");
    assert_eq!(entries[0].oid, two);
    assert_eq!(entries[1].oid, two);
    assert_eq!(entries[1].previous_oid, three);
    assert_eq!(entries[2].oid, three);
    assert!(
        entries
            .iter()
            .all(|x| x.available && x.timestamp > 1_000_000_000)
    );
    assert!(entries[0].message.contains("moving from main to other"));
    let branch = e.git.list_reflog(&e.info, "refs/heads/main").await.unwrap();
    assert_eq!(branch[0].action, "reset");
    assert_eq!(branch[0].selector, "main@{0}");
}

#[tokio::test]
async fn an_old_commit_is_inspected_with_what_still_reaches_it() {
    let e = env().await;
    let three = e.out(&["rev-parse", "HEAD"]);
    run_git(e.path(), &["reset", "-q", "--hard", "HEAD~2"]);
    let inspected = e.git.inspect_reflog_entry(&e.info, &three).await.unwrap();
    assert!(inspected.available);
    assert_eq!(inspected.subject, "three");
    assert_eq!(inspected.parents.len(), 1);
    assert!(
        inspected.reachable_from.is_empty(),
        "{:?}",
        inspected.reachable_from
    );
    run_git(e.path(), &["branch", "keeper", &three]);
    let again = e.git.inspect_reflog_entry(&e.info, &three).await.unwrap();
    assert_eq!(again.reachable_from, ["keeper"]);
}

#[tokio::test]
async fn a_recovery_branch_finds_the_commit_without_moving_the_current_branch() {
    let e = env().await;
    let three = e.out(&["rev-parse", "HEAD"]);
    run_git(e.path(), &["reset", "-q", "--hard", "HEAD~2"]);
    let head = e.out(&["rev-parse", "HEAD"]);
    e.git
        .create_recovery_branch(&e.info, &three, "recup-three")
        .await
        .unwrap();
    assert_eq!(e.out(&["rev-parse", "refs/heads/recup-three"]), three);
    assert_eq!(e.out(&["rev-parse", "HEAD"]), head);
    assert_eq!(e.out(&["symbolic-ref", "--short", "HEAD"]), "main");
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "1\n"
    );
    assert_eq!(
        e.git
            .create_recovery_branch(&e.info, &three, "recup-three")
            .await
            .unwrap_err(),
        ReflogError::Branch(BranchError::AlreadyExists)
    );
    assert_eq!(
        e.git
            .create_recovery_branch(&e.info, &three, "bad name")
            .await
            .unwrap_err(),
        ReflogError::Branch(BranchError::InvalidName)
    );
}

#[tokio::test]
async fn an_entry_whose_object_is_gone_is_told_apart_from_a_successful_recovery() {
    let e = env().await;
    let three = e.out(&["rev-parse", "HEAD"]);
    run_git(e.path(), &["reset", "-q", "--hard", "HEAD~1"]);
    run_git(
        e.path(),
        &["reflog", "expire", "--expire-unreachable=now", "--all"],
    );
    // remove the loose object to simulate an expired one
    let object = e
        .path()
        .join(".git/objects")
        .join(&three[..2])
        .join(&three[2..]);
    let _ = run_git(e.path(), &["repack", "-q", "-a", "-d"]);
    let _ = std::fs::remove_file(&object);
    run_git(e.path(), &["prune", "--expire=now"]);
    let entries = e.git.list_reflog(&e.info, "HEAD").await.unwrap();
    let gone = entries.iter().find(|x| x.oid == three);
    if let Some(gone) = gone {
        assert!(!gone.available);
    }
    let inspected = e.git.inspect_reflog_entry(&e.info, &three).await.unwrap();
    assert!(!inspected.available);
    assert_eq!(
        e.git
            .create_recovery_branch(&e.info, &three, "recup")
            .await
            .unwrap_err(),
        ReflogError::ObjectUnavailable
    );
    assert!(e.out(&["for-each-ref", "refs/heads/recup"]).is_empty());
}

#[tokio::test]
async fn a_fabricated_reflog_line_pointing_at_a_missing_object_is_listed_as_unavailable() {
    let e = env().await;
    let log = e.path().join(".git/logs/HEAD");
    let mut text = std::fs::read_to_string(&log).unwrap();
    let ghost = "ab".repeat(20);
    let tip = e.out(&["rev-parse", "HEAD"]);
    text.push_str(&format!(
        "{tip} {ghost} t <t@example.com> 1700000000 +0000\tcommit: ghost\n"
    ));
    std::fs::write(&log, text).unwrap();
    let entries = e.git.list_reflog(&e.info, "HEAD").await.unwrap();
    assert_eq!(entries[0].oid, ghost);
    assert!(!entries[0].available);
    assert!(entries[1].available);
    assert_eq!(
        e.git
            .create_recovery_branch(&e.info, &ghost, "ghosty")
            .await
            .unwrap_err(),
        ReflogError::ObjectUnavailable
    );
}

#[tokio::test]
async fn hostile_references_and_ids_are_refused() {
    let e = env().await;
    for reference in [
        "refs/tags/x",
        "--all",
        "HEAD~1",
        "refs/heads/",
        "refs/heads/-x",
        "../x",
    ] {
        assert_eq!(
            e.git.list_reflog(&e.info, reference).await.unwrap_err(),
            ReflogError::InvalidReference,
            "{reference}"
        );
    }
    for oid in ["HEAD", "--output=x", "abc", "zz".repeat(20).as_str()] {
        assert_eq!(
            e.git.inspect_reflog_entry(&e.info, oid).await.unwrap_err(),
            ReflogError::InvalidReference
        );
    }
}

#[tokio::test]
async fn a_repository_without_a_reflog_lists_nothing() {
    let e = env().await;
    assert!(
        e.git
            .list_reflog(&e.info, "refs/heads/never-existed")
            .await
            .unwrap()
            .is_empty()
    );
}
