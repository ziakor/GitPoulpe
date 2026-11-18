mod common;

use std::path::{Path, PathBuf};

use common::{commit_all, git as run_git, git_output, init_repo, write};
use gitpoulpe_core::{MergeKind, MergeMode, MergeOutcome, Profile, SessionInfo, SigningPreference};
use gitpoulpe_git::{Cancel, Git, MergeError};

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
    fn head(&self) -> String {
        self.out(&["rev-parse", "HEAD"])
    }
}

async fn env_with(setup: impl FnOnce(&Path)) -> Env {
    let repo = init_repo();
    setup(repo.path());
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com");
    let info = git.open(repo.path(), &Cancel::new()).await.unwrap();
    Env { repo, git, info }
}

fn linear(p: &Path) {
    write(p, "a.txt", "base\n");
    commit_all(p, "base");
    run_git(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "f.txt", "feature\n");
    commit_all(p, "feature work");
    run_git(p, &["checkout", "-q", "main"]);
}

fn divergent(p: &Path) {
    linear(p);
    write(p, "m.txt", "main\n");
    commit_all(p, "main work");
}

fn conflicting(p: &Path) {
    write(p, "a.txt", "base\n");
    commit_all(p, "base");
    run_git(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", "feature\n");
    commit_all(p, "feature");
    run_git(p, &["checkout", "-q", "main"]);
    write(p, "a.txt", "main\n");
    commit_all(p, "main");
}

#[tokio::test]
async fn the_preview_tells_fast_forward_true_merge_and_up_to_date_apart() {
    let e = env_with(linear).await;
    let preview = e.git.merge_preview(&e.info, "feature").await.unwrap();
    assert_eq!(preview.kind, MergeKind::FastForward);
    assert_eq!((preview.only_in_head, preview.only_in_source), (0, 1));
    assert_eq!(preview.current.as_deref(), Some("main"));
    assert_eq!(preview.head, e.head());
    let d = env_with(divergent).await;
    let preview = d.git.merge_preview(&d.info, "feature").await.unwrap();
    assert_eq!(preview.kind, MergeKind::TrueMerge);
    assert_eq!((preview.only_in_head, preview.only_in_source), (1, 1));
    assert_eq!(preview.predicted_conflicts, Some(Vec::new()));
    run_git(d.path(), &["merge", "-q", "feature"]);
    let preview = d.git.merge_preview(&d.info, "feature").await.unwrap();
    assert_eq!(preview.kind, MergeKind::UpToDate);
}

#[tokio::test]
async fn conflicts_are_predicted_without_touching_the_work() {
    let e = env_with(conflicting).await;
    let before = (e.head(), e.out(&["status", "--porcelain"]));
    let preview = e.git.merge_preview(&e.info, "feature").await.unwrap();
    assert_eq!(
        preview.predicted_conflicts,
        Some(vec![PathBuf::from("a.txt")])
    );
    assert_eq!((e.head(), e.out(&["status", "--porcelain"])), before);
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "main\n"
    );
}

#[tokio::test]
async fn a_linear_merge_fast_forwards_and_no_ff_still_makes_a_merge_commit() {
    let e = env_with(linear).await;
    let head = e.head();
    let outcome = e
        .git
        .merge_branch(&e.info, "feature", MergeMode::Auto, &head, false, None)
        .await
        .unwrap();
    let feature = e.out(&["rev-parse", "feature"]);
    assert_eq!(
        outcome,
        MergeOutcome::FastForward {
            from: head.clone(),
            to: feature.clone()
        }
    );
    assert_eq!(e.head(), feature);
    let n = env_with(linear).await;
    let head = n.head();
    let outcome = n
        .git
        .merge_branch(
            &n.info,
            "feature",
            MergeMode::NoFastForward,
            &head,
            false,
            None,
        )
        .await
        .unwrap();
    assert!(matches!(outcome, MergeOutcome::Merged { .. }));
    assert_eq!(
        n.out(&["rev-list", "--parents", "-n", "1", "HEAD"])
            .split(' ')
            .count(),
        3
    );
}

#[tokio::test]
async fn a_divergent_merge_matches_what_git_itself_produces() {
    let e = env_with(divergent).await;
    let oracle = tempfile::tempdir().unwrap();
    run_git(
        oracle.path(),
        &["clone", "-q", &e.path().display().to_string(), "."],
    );
    run_git(
        oracle.path(),
        &["merge", "-q", "--no-edit", "origin/feature"],
    );
    let expected_tree =
        String::from_utf8_lossy(&run_git(oracle.path(), &["rev-parse", "HEAD^{tree}"]))
            .trim()
            .to_owned();
    let head = e.head();
    let outcome = e
        .git
        .merge_branch(&e.info, "feature", MergeMode::Auto, &head, false, None)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        MergeOutcome::Merged {
            stash_kept: false,
            ..
        }
    ));
    assert_eq!(e.out(&["rev-parse", "HEAD^{tree}"]), expected_tree);
    assert!(
        e.out(&["log", "-1", "--format=%s"])
            .starts_with("Merge branch 'feature'")
    );
    assert_eq!(e.out(&["rev-parse", "HEAD^1"]), head);
    assert_eq!(
        e.out(&["rev-parse", "HEAD^2"]),
        e.out(&["rev-parse", "feature"])
    );
}

#[tokio::test]
async fn the_merge_commit_carries_the_profile_identity() {
    let e = env_with(divergent).await;
    let profile = Profile {
        id: "p".into(),
        label: "P".into(),
        user_name: "Grace Hopper".into(),
        user_email: "grace@example.org".into(),
        signing: SigningPreference::None,
    };
    let head = e.head();
    e.git
        .merge_branch(
            &e.info,
            "feature",
            MergeMode::Auto,
            &head,
            false,
            Some(&profile),
        )
        .await
        .unwrap();
    assert_eq!(
        e.out(&["log", "-1", "--format=%an <%ae>|%cn <%ce>"]),
        "Grace Hopper <grace@example.org>|Grace Hopper <grace@example.org>"
    );
}

#[tokio::test]
async fn a_merging_conflict_opens_the_conflict_state_and_can_be_abandoned() {
    let e = env_with(conflicting).await;
    let head = e.head();
    let outcome = e
        .git
        .merge_branch(&e.info, "feature", MergeMode::Auto, &head, false, None)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        MergeOutcome::Conflicts {
            files: vec![PathBuf::from("a.txt")]
        }
    );
    let op = e.git.operation_state(&e.info).await.unwrap().unwrap();
    assert!(op.theirs_label.contains("feature"));
    e.git.abort_operation(&e.info, &[]).await.unwrap();
    assert_eq!(e.head(), head);
    assert!(e.out(&["status", "--porcelain"]).is_empty());
}

#[tokio::test]
async fn local_changes_that_would_be_overwritten_stop_the_merge_and_stay_intact() {
    let e = env_with(|p| {
        write(p, "a.txt", "base\n");
        write(p, "other.txt", "o\n");
        commit_all(p, "base");
        run_git(p, &["checkout", "-q", "-b", "feature"]);
        write(p, "a.txt", "feature\n");
        commit_all(p, "feature");
        run_git(p, &["checkout", "-q", "main"]);
        write(p, "other.txt", "o main\n");
        commit_all(p, "main");
    })
    .await;
    write(e.path(), "a.txt", "my unsaved work\n");
    let head = e.head();
    let error = e
        .git
        .merge_branch(&e.info, "feature", MergeMode::Auto, &head, false, None)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        MergeError::LocalChanges(vec![PathBuf::from("a.txt")])
    );
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "my unsaved work\n"
    );
    assert_eq!(e.head(), head);
    let preview = e.git.merge_preview(&e.info, "feature").await.unwrap();
    assert_eq!(preview.local_changes, vec![PathBuf::from("a.txt")]);
}

#[tokio::test]
async fn unrelated_local_changes_survive_a_merge() {
    let e = env_with(divergent).await;
    write(e.path(), "a.txt", "edited locally\n");
    let head = e.head();
    e.git
        .merge_branch(&e.info, "feature", MergeMode::Auto, &head, false, None)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "edited locally\n"
    );
    assert!(e.out(&["status", "--porcelain"]).contains("a.txt"));
}

#[tokio::test]
async fn autostash_keeps_overlapping_work_retrievable_in_the_stash() {
    let e = env_with(|p| {
        write(p, "a.txt", "base\n");
        commit_all(p, "base");
        run_git(p, &["checkout", "-q", "-b", "feature"]);
        write(p, "a.txt", "feature\n");
        commit_all(p, "feature");
        run_git(p, &["checkout", "-q", "main"]);
        write(p, "b.txt", "b\n");
        commit_all(p, "main");
    })
    .await;
    write(e.path(), "a.txt", "my unsaved work\n");
    let head = e.head();
    let outcome = e
        .git
        .merge_branch(&e.info, "feature", MergeMode::Auto, &head, true, None)
        .await
        .unwrap();
    assert!(
        matches!(
            outcome,
            MergeOutcome::Merged {
                stash_kept: true,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(e.out(&["stash", "list"]).lines().count(), 1);
    assert!(
        e.out(&["stash", "show", "-p", "stash@{0}"])
            .contains("my unsaved work")
    );
}

#[tokio::test]
async fn bad_requests_are_refused_without_touching_the_repository() {
    let e = env_with(divergent).await;
    let head = e.head();
    let go = |source: &'static str, mode, expected: String| {
        let (git, info) = (e.git.clone(), e.info.clone());
        async move {
            git.merge_branch(&info, source, mode, &expected, false, None)
                .await
        }
    };
    assert_eq!(
        go("ghost", MergeMode::Auto, head.clone())
            .await
            .unwrap_err(),
        MergeError::InvalidSource
    );
    assert_eq!(
        go("-x", MergeMode::Auto, head.clone()).await.unwrap_err(),
        MergeError::InvalidSource
    );
    assert_eq!(
        go("feature", MergeMode::Auto, "0".repeat(40))
            .await
            .unwrap_err(),
        MergeError::HeadMoved
    );
    assert_eq!(
        go("feature", MergeMode::FastForwardOnly, head.clone())
            .await
            .unwrap_err(),
        MergeError::CannotFastForward
    );
    assert_eq!(
        e.git.merge_preview(&e.info, "main").await.unwrap_err(),
        MergeError::SameBranch
    );
    assert_eq!(e.head(), head);
    assert!(e.out(&["status", "--porcelain"]).is_empty());
}

#[tokio::test]
async fn another_operation_in_progress_blocks_a_new_merge() {
    let e = env_with(conflicting).await;
    let head = e.head();
    assert!(!git_output(e.path(), &["merge", "feature"]).status.success());
    let error = e
        .git
        .merge_branch(&e.info, "feature", MergeMode::Auto, &head, false, None)
        .await
        .unwrap_err();
    assert_eq!(error, MergeError::OperationInProgress);
}

#[tokio::test]
async fn an_unborn_branch_has_nothing_to_merge_into() {
    let e = env_with(|_| {}).await;
    assert_eq!(
        e.git.merge_preview(&e.info, "main").await.unwrap_err(),
        MergeError::NothingCommitted
    );
}
