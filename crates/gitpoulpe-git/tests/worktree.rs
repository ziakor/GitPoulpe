mod common;

use std::path::{Path, PathBuf};

use common::{commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{BranchChoice, Cancel, Git, WorktreeError};

struct Env {
    repo: tempfile::TempDir,
    area: tempfile::TempDir,
    git: Git,
    info: SessionInfo,
}

impl Env {
    fn path(&self) -> &Path {
        self.repo.path()
    }
    fn out(&self, dir: &Path, args: &[&str]) -> String {
        String::from_utf8_lossy(&run_git(dir, args))
            .trim()
            .to_owned()
    }
    fn wt(&self, name: &str) -> PathBuf {
        std::fs::canonicalize(self.area.path()).unwrap().join(name)
    }
}

async fn env() -> Env {
    let repo = init_repo();
    write(repo.path(), "a.txt", "one\n");
    commit_all(repo.path(), "one");
    run_git(repo.path(), &["branch", "feature"]);
    run_git(repo.path(), &["branch", "spare"]);
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com");
    let info = git.open(repo.path(), &Cancel::new()).await.unwrap();
    Env {
        repo,
        area: tempfile::tempdir().unwrap(),
        git,
        info,
    }
}

#[tokio::test]
async fn each_worktree_shows_its_path_branch_and_its_own_state() {
    let e = env().await;
    let feature = e.wt("feature");
    e.git
        .add_worktree(&e.info, &feature, BranchChoice::Existing("feature"))
        .await
        .unwrap();
    write(&feature, "a.txt", "changed in the feature worktree\n");
    write(&feature, "new.txt", "untracked there\n");
    write(e.path(), "main-only.txt", "x\n");
    run_git(e.path(), &["add", "main-only.txt"]);
    let list = e.git.list_worktrees(&e.info).await.unwrap();
    assert_eq!(list.len(), 2);
    let main = &list[0];
    assert!(main.main && main.current);
    assert_eq!(main.branch.as_deref(), Some("main"));
    assert_eq!((main.staged, main.unstaged, main.untracked), (1, 0, 0));
    let other = &list[1];
    assert_eq!(other.path, feature);
    assert_eq!(other.branch.as_deref(), Some("feature"));
    assert!(!other.main && !other.current);
    assert_eq!((other.staged, other.unstaged, other.untracked), (0, 1, 1));
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "one\n"
    );
}

#[tokio::test]
async fn creating_a_worktree_for_a_new_or_existing_branch_leaves_the_others_alone() {
    let e = env().await;
    write(e.path(), "a.txt", "dirty main\n");
    let new = e.wt("new");
    e.git
        .add_worktree(
            &e.info,
            &new,
            BranchChoice::New {
                name: "topic",
                start: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(e.out(&new, &["symbolic-ref", "--short", "HEAD"]), "topic");
    let spare = e.wt("spare");
    e.git
        .add_worktree(&e.info, &spare, BranchChoice::Existing("spare"))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "dirty main\n"
    );
    assert_eq!(std::fs::read_to_string(new.join("a.txt")).unwrap(), "one\n");
    let head = e.out(e.path(), &["rev-parse", "HEAD"]);
    let started = e.wt("started");
    e.git
        .add_worktree(
            &e.info,
            &started,
            BranchChoice::New {
                name: "from-oid",
                start: Some(&head),
            },
        )
        .await
        .unwrap();
    assert_eq!(e.out(&started, &["rev-parse", "HEAD"]), head);
}

#[tokio::test]
async fn collisions_and_unsafe_requests_are_refused() {
    let e = env().await;
    let first = e.wt("first");
    e.git
        .add_worktree(&e.info, &first, BranchChoice::Existing("feature"))
        .await
        .unwrap();
    let again = e.wt("again");
    assert!(matches!(
        e.git.add_worktree(&e.info, &again, BranchChoice::Existing("feature")).await.unwrap_err(),
        WorktreeError::BranchInUse(p) if p == first
    ));
    assert!(matches!(
        e.git
            .add_worktree(&e.info, &e.wt("main-again"), BranchChoice::Existing("main"))
            .await
            .unwrap_err(),
        WorktreeError::BranchInUse(_)
    ));
    assert_eq!(
        e.git
            .add_worktree(&e.info, &first, BranchChoice::Existing("spare"))
            .await
            .unwrap_err(),
        WorktreeError::PathExists
    );
    assert_eq!(
        e.git
            .add_worktree(
                &e.info,
                &e.path().join("inside"),
                BranchChoice::Existing("spare")
            )
            .await
            .unwrap_err(),
        WorktreeError::InsideCurrent
    );
    assert_eq!(
        e.git
            .add_worktree(
                &e.info,
                Path::new("relative"),
                BranchChoice::Existing("spare")
            )
            .await
            .unwrap_err(),
        WorktreeError::InvalidPath
    );
    assert_eq!(
        e.git
            .add_worktree(&e.info, &e.wt("x/../y"), BranchChoice::Existing("spare"))
            .await
            .unwrap_err(),
        WorktreeError::InvalidPath
    );
    assert_eq!(
        e.git
            .add_worktree(
                &e.info,
                &e.wt("nodir/deeper"),
                BranchChoice::Existing("spare")
            )
            .await
            .unwrap_err(),
        WorktreeError::ParentMissing
    );
    assert_eq!(
        e.git
            .add_worktree(&e.info, &e.wt("t1"), BranchChoice::Existing("ghost"))
            .await
            .unwrap_err(),
        WorktreeError::NoSuchBranch
    );
    assert_eq!(
        e.git
            .add_worktree(&e.info, &e.wt("t2"), BranchChoice::Existing("bad name"))
            .await
            .unwrap_err(),
        WorktreeError::InvalidBranch
    );
    assert_eq!(
        e.git
            .add_worktree(
                &e.info,
                &e.wt("t3"),
                BranchChoice::New {
                    name: "spare",
                    start: None
                }
            )
            .await
            .unwrap_err(),
        WorktreeError::BranchExists
    );
    assert_eq!(e.git.list_worktrees(&e.info).await.unwrap().len(), 2);
}

#[tokio::test]
async fn locking_protects_a_worktree_from_removal_until_it_is_unlocked() {
    let e = env().await;
    let wt = e.wt("locked");
    e.git
        .add_worktree(&e.info, &wt, BranchChoice::Existing("feature"))
        .await
        .unwrap();
    e.git
        .lock_worktree(&e.info, &wt, "on a removable disk")
        .await
        .unwrap();
    let list = e.git.list_worktrees(&e.info).await.unwrap();
    assert_eq!(list[1].locked.as_deref(), Some("on a removable disk"));
    assert_eq!(
        e.git
            .lock_worktree(&e.info, &wt, "again")
            .await
            .unwrap_err(),
        WorktreeError::AlreadyLocked
    );
    assert!(matches!(
        e.git.remove_worktree(&e.info, &wt, true).await.unwrap_err(),
        WorktreeError::Locked(reason) if reason == "on a removable disk"
    ));
    assert!(wt.exists());
    e.git.unlock_worktree(&e.info, &wt).await.unwrap();
    assert_eq!(
        e.git.unlock_worktree(&e.info, &wt).await.unwrap_err(),
        WorktreeError::NotLocked
    );
    e.git.remove_worktree(&e.info, &wt, false).await.unwrap();
    assert!(!wt.exists());
}

#[tokio::test]
async fn removing_a_modified_worktree_needs_a_decision_and_keeps_a_snapshot_of_the_work() {
    let e = env().await;
    let wt = e.wt("work");
    e.git
        .add_worktree(&e.info, &wt, BranchChoice::Existing("feature"))
        .await
        .unwrap();
    write(&wt, "a.txt", "precious tracked edit\n");
    write(&wt, "scratch.txt", "untracked scratch\n");
    let preview = e.git.worktree_removal(&e.info, &wt).await.unwrap();
    assert_eq!(preview.changed, [PathBuf::from("a.txt")]);
    assert_eq!(preview.untracked, [PathBuf::from("scratch.txt")]);
    let error = e
        .git
        .remove_worktree(&e.info, &wt, false)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, WorktreeError::NeedsDecision(r) if r.changed.len() == 1 && r.untracked.len() == 1),
        "{error:?}"
    );
    assert!(wt.join("a.txt").exists() && wt.join("scratch.txt").exists());
    let removed = e.git.remove_worktree(&e.info, &wt, true).await.unwrap();
    assert!(!wt.exists());
    assert_eq!(removed.lost_untracked, 1);
    let snapshot = removed.snapshot.expect("tracked work was snapshotted");
    assert_eq!(
        e.out(e.path(), &["show", &format!("{snapshot}:a.txt")]),
        "precious tracked edit"
    );
    assert!(
        e.out(e.path(), &["for-each-ref", "refs/heads/feature"])
            .contains("feature")
    );
}

#[tokio::test]
async fn the_main_and_the_open_worktree_cannot_be_removed_and_unknown_ones_are_refused() {
    let e = env().await;
    assert_eq!(
        e.git
            .remove_worktree(&e.info, e.path(), true)
            .await
            .unwrap_err(),
        WorktreeError::IsMain
    );
    let wt = e.wt("other");
    e.git
        .add_worktree(&e.info, &wt, BranchChoice::Existing("feature"))
        .await
        .unwrap();
    let other_info = e.git.open(&wt, &Cancel::new()).await.unwrap();
    assert_eq!(
        e.git
            .remove_worktree(&other_info, &wt, true)
            .await
            .unwrap_err(),
        WorktreeError::IsCurrent
    );
    assert!(wt.exists());
    assert_eq!(
        e.git
            .remove_worktree(&e.info, &e.wt("ghost"), true)
            .await
            .unwrap_err(),
        WorktreeError::NoSuchWorktree
    );
    assert_eq!(
        e.git
            .lock_worktree(&e.info, e.path(), "x")
            .await
            .unwrap_err(),
        WorktreeError::IsMain
    );
    let list = e.git.list_worktrees(&other_info).await.unwrap();
    assert!(list.iter().find(|w| w.path == wt).unwrap().current);
    assert!(!list[0].current);
}

#[tokio::test]
async fn a_worktree_whose_directory_disappeared_is_listed_as_prunable_and_pruned_on_request() {
    let e = env().await;
    let wt = e.wt("vanishing");
    e.git
        .add_worktree(&e.info, &wt, BranchChoice::Existing("feature"))
        .await
        .unwrap();
    std::fs::remove_dir_all(&wt).unwrap();
    let list = e.git.list_worktrees(&e.info).await.unwrap();
    assert_eq!(list.len(), 2);
    assert!(list[1].prunable.is_some());
    assert!(!list[1].dirty());
    e.git.prune_worktrees(&e.info).await.unwrap();
    assert_eq!(e.git.list_worktrees(&e.info).await.unwrap().len(), 1);
}
