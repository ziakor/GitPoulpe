mod common;

use std::path::{Path, PathBuf};

use common::{commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::{
    ConflictKind, OperationKind, Profile, SessionInfo, SigningPreference, StashApplied, StashScope,
};
use gitpoulpe_git::{Cancel, ConflictSide, Git, StashError};

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
    fn status(&self) -> String {
        self.out(&["status", "--porcelain=v1"])
    }
}

async fn env() -> Env {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "a base\n");
    write(p, "b.txt", "b base\n");
    write(p, "c.txt", "c base\n");
    commit_all(p, "base");
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com");
    let info = git.open(p, &Cancel::new()).await.unwrap();
    Env { repo, git, info }
}

fn mixed_work(e: &Env) {
    write(e.path(), "a.txt", "a staged\n");
    run_git(e.path(), &["add", "a.txt"]);
    write(e.path(), "b.txt", "b unstaged\n");
    write(e.path(), "new.txt", "untracked\n");
}

#[tokio::test]
async fn the_default_scope_keeps_staged_and_unstaged_changes_and_leaves_untracked_files() {
    let e = env().await;
    mixed_work(&e);
    let stash = e
        .git
        .create_stash(&e.info, Some("my work"), StashScope::default(), None)
        .await
        .unwrap();
    assert_eq!(stash.message, "my work");
    assert_eq!(stash.branch.as_deref(), Some("main"));
    assert_eq!(e.status(), "?? new.txt");
    assert_eq!(
        e.out(&["show", &format!("{}:a.txt", stash.oid)]),
        "a staged"
    );
    assert_eq!(
        e.out(&["show", &format!("{}:b.txt", stash.oid)]),
        "b unstaged"
    );
    let detail = e.git.stash_detail(&e.info, &stash.oid).await.unwrap();
    let names: Vec<(String, bool)> = detail
        .files
        .iter()
        .map(|f| (f.path.display().to_string(), f.staged))
        .collect();
    assert_eq!(
        names,
        [("a.txt".to_owned(), true), ("b.txt".to_owned(), false)]
    );
    assert!(detail.untracked.is_empty());
}

#[tokio::test]
async fn untracked_files_are_stashed_only_when_asked() {
    let e = env().await;
    mixed_work(&e);
    let scope = StashScope {
        include_untracked: true,
        ..StashScope::default()
    };
    let stash = e
        .git
        .create_stash(&e.info, None, scope, None)
        .await
        .unwrap();
    assert!(e.status().is_empty());
    let detail = e.git.stash_detail(&e.info, &stash.oid).await.unwrap();
    assert_eq!(detail.untracked, [PathBuf::from("new.txt")]);
    assert_eq!(
        e.out(&["show", &format!("{}^3:new.txt", stash.oid)]),
        "untracked"
    );
    assert!(stash.message.contains("main") || stash.branch.as_deref() == Some("main"));
}

#[tokio::test]
async fn keeping_the_index_leaves_staged_changes_in_place() {
    let e = env().await;
    mixed_work(&e);
    let scope = StashScope {
        keep_index: true,
        ..StashScope::default()
    };
    e.git
        .create_stash(&e.info, None, scope, None)
        .await
        .unwrap();
    assert_eq!(e.out(&["diff", "--cached", "--name-only"]), "a.txt");
    assert_eq!(
        std::fs::read_to_string(e.path().join("b.txt")).unwrap(),
        "b base\n"
    );
}

#[tokio::test]
async fn staged_only_stashes_the_index_and_keeps_the_rest_of_the_work() {
    let e = env().await;
    mixed_work(&e);
    let scope = StashScope {
        staged_only: true,
        ..StashScope::default()
    };
    let stash = e
        .git
        .create_stash(&e.info, None, scope, None)
        .await
        .unwrap();
    assert!(e.out(&["diff", "--cached", "--name-only"]).is_empty());
    assert_eq!(
        std::fs::read_to_string(e.path().join("b.txt")).unwrap(),
        "b unstaged\n"
    );
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "a base\n"
    );
    let detail = e.git.stash_detail(&e.info, &stash.oid).await.unwrap();
    assert_eq!(detail.files.len(), 1);
    assert_eq!(
        e.git
            .create_stash(
                &e.info,
                None,
                StashScope {
                    staged_only: true,
                    include_untracked: true,
                    keep_index: false
                },
                None
            )
            .await
            .unwrap_err(),
        StashError::InvalidScope
    );
}

#[tokio::test]
async fn there_is_nothing_to_stash_in_a_clean_tree() {
    let e = env().await;
    assert_eq!(
        e.git
            .create_stash(&e.info, None, StashScope::default(), None)
            .await
            .unwrap_err(),
        StashError::NothingToStash
    );
    write(e.path(), "new.txt", "u\n");
    assert_eq!(
        e.git
            .create_stash(&e.info, None, StashScope::default(), None)
            .await
            .unwrap_err(),
        StashError::NothingToStash
    );
    assert_eq!(e.git.list_stashes(&e.info).await.unwrap().len(), 0);
    std::fs::remove_file(e.path().join("new.txt")).unwrap();
    write(e.path(), "a.txt", "work\n");
    e.git
        .create_stash(&e.info, None, StashScope::default(), None)
        .await
        .unwrap();
    assert_eq!(
        e.git
            .create_stash(&e.info, None, StashScope::default(), None)
            .await
            .unwrap_err(),
        StashError::NothingToStash,
        "an older stash must not be reported as the new one"
    );
    assert_eq!(e.git.list_stashes(&e.info).await.unwrap().len(), 1);
}

#[tokio::test]
async fn the_stash_commit_carries_the_profile_identity() {
    let e = env().await;
    write(e.path(), "b.txt", "x\n");
    let profile = Profile {
        id: "p".into(),
        label: "P".into(),
        user_name: "Grace Hopper".into(),
        user_email: "grace@example.org".into(),
        signing: SigningPreference::None,
    };
    let stash = e
        .git
        .create_stash(&e.info, None, StashScope::default(), Some(&profile))
        .await
        .unwrap();
    assert_eq!(
        e.out(&["log", "-1", "--format=%an <%ae>", &stash.oid]),
        "Grace Hopper <grace@example.org>"
    );
}

#[tokio::test]
async fn apply_keeps_the_stash_and_pop_removes_it_and_the_index_can_be_restored() {
    let e = env().await;
    mixed_work(&e);
    let stash = e
        .git
        .create_stash(&e.info, None, StashScope::default(), None)
        .await
        .unwrap();
    let applied = e
        .git
        .apply_stash(&e.info, &stash.oid, false, true)
        .await
        .unwrap();
    assert_eq!(applied, StashApplied::Clean { dropped: false });
    assert_eq!(e.out(&["diff", "--cached", "--name-only"]), "a.txt");
    assert_eq!(
        std::fs::read_to_string(e.path().join("b.txt")).unwrap(),
        "b unstaged\n"
    );
    assert_eq!(e.git.list_stashes(&e.info).await.unwrap().len(), 1);
    run_git(e.path(), &["checkout", "-q", "--", "b.txt"]);
    run_git(e.path(), &["reset", "-q", "--hard"]);
    let popped = e
        .git
        .apply_stash(&e.info, &stash.oid, true, false)
        .await
        .unwrap();
    assert_eq!(popped, StashApplied::Clean { dropped: true });
    assert!(e.git.list_stashes(&e.info).await.unwrap().is_empty());
    assert!(e.out(&["diff", "--cached", "--name-only"]).is_empty());
    assert!(e.status().contains("M a.txt") && e.status().contains("M b.txt"));
}

#[tokio::test]
async fn a_conflicting_application_keeps_the_stash_and_leaves_resolvable_conflicts() {
    let e = env().await;
    write(e.path(), "a.txt", "stashed change\n");
    let stash = e
        .git
        .create_stash(&e.info, None, StashScope::default(), None)
        .await
        .unwrap();
    write(e.path(), "a.txt", "committed elsewhere\n");
    commit_all(e.path(), "diverge");
    let outcome = e
        .git
        .apply_stash(&e.info, &stash.oid, true, false)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        StashApplied::Conflicts {
            files: vec![PathBuf::from("a.txt")]
        }
    );
    assert_eq!(
        e.git.list_stashes(&e.info).await.unwrap().len(),
        1,
        "pop must keep the stash on conflict"
    );
    let op = e.git.operation_state(&e.info).await.unwrap().unwrap();
    assert_eq!(op.kind, OperationKind::StashApply);
    let files = e.git.list_conflicts(&e.info).await.unwrap();
    assert_eq!(files[0].kind, ConflictKind::BothModified);
    e.git
        .resolve_with_content(&e.info, Path::new("a.txt"), b"resolved\n", false)
        .await
        .unwrap();
    assert!(e.git.operation_state(&e.info).await.unwrap().is_none());
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "resolved\n"
    );
    assert_eq!(e.git.list_stashes(&e.info).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_stash_application_cannot_be_continued_or_aborted_like_a_merge() {
    let e = env().await;
    write(e.path(), "a.txt", "stashed\n");
    let stash = e
        .git
        .create_stash(&e.info, None, StashScope::default(), None)
        .await
        .unwrap();
    write(e.path(), "a.txt", "other\n");
    commit_all(e.path(), "diverge");
    e.git
        .apply_stash(&e.info, &stash.oid, false, false)
        .await
        .unwrap();
    assert!(e.git.continue_operation(&e.info, None, &[]).await.is_err());
    assert!(e.git.abort_operation(&e.info, &[]).await.is_err());
    e.git
        .resolve_with_side(&e.info, Path::new("a.txt"), ConflictSide::Ours)
        .await
        .unwrap();
    assert!(e.git.operation_state(&e.info).await.unwrap().is_none());
}

#[tokio::test]
async fn actions_target_a_stash_by_identity_not_by_position() {
    let e = env().await;
    write(e.path(), "a.txt", "first stash\n");
    let first = e
        .git
        .create_stash(&e.info, Some("first"), StashScope::default(), None)
        .await
        .unwrap();
    write(e.path(), "b.txt", "second stash\n");
    let second = e
        .git
        .create_stash(&e.info, Some("second"), StashScope::default(), None)
        .await
        .unwrap();
    assert_eq!(second.index, 0);
    let listed = e.git.list_stashes(&e.info).await.unwrap();
    assert_eq!(listed[1].oid, first.oid);
    e.git
        .apply_stash(&e.info, &first.oid, false, false)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "first stash\n"
    );
    assert_eq!(
        std::fs::read_to_string(e.path().join("b.txt")).unwrap(),
        "b base\n"
    );
    assert_eq!(
        e.git
            .apply_stash(&e.info, &"0".repeat(40), false, false)
            .await
            .unwrap_err(),
        StashError::NoSuchStash
    );
    assert_eq!(
        e.git.drop_stash(&e.info, "stash@{0}").await.unwrap_err(),
        StashError::NoSuchStash
    );
}

#[tokio::test]
async fn dropping_needs_the_exact_stash_and_the_work_stays_recoverable() {
    let e = env().await;
    write(e.path(), "a.txt", "precious\n");
    let stash = e
        .git
        .create_stash(&e.info, Some("keep me"), StashScope::default(), None)
        .await
        .unwrap();
    let dropped = e.git.drop_stash(&e.info, &stash.oid).await.unwrap();
    assert_eq!(dropped.oid, stash.oid);
    assert!(e.git.list_stashes(&e.info).await.unwrap().is_empty());
    run_git(e.path(), &["stash", "store", "-m", "recovered", &stash.oid]);
    let again = e.git.list_stashes(&e.info).await.unwrap();
    assert_eq!(again.len(), 1);
    e.git
        .apply_stash(&e.info, &again[0].oid, false, false)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "precious\n"
    );
}

#[tokio::test]
async fn no_stash_is_created_while_another_operation_is_in_progress() {
    let e = env().await;
    write(e.path(), "a.txt", "main\n");
    commit_all(e.path(), "main change");
    run_git(e.path(), &["checkout", "-q", "-b", "side", "HEAD~1"]);
    write(e.path(), "a.txt", "side\n");
    commit_all(e.path(), "side change");
    assert!(
        !common::git_output(e.path(), &["merge", "main"])
            .status
            .success()
    );
    assert_eq!(
        e.git
            .create_stash(&e.info, None, StashScope::default(), None)
            .await
            .unwrap_err(),
        StashError::OperationInProgress
    );
}
