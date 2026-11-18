mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{commit_all, git as run_git, write};
use gitpoulpe_core::{Integration, PullKind, PullOutcome, PullStrategy, SessionInfo};
use gitpoulpe_git::{Cancel, Git, NetError, PullError, PullRequest};

struct Env {
    dir: tempfile::TempDir,
    bare: tempfile::TempDir,
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
    fn actor(&self, file: &str, content: &str) -> String {
        let clone = tempfile::tempdir().unwrap();
        run_git(
            clone.path(),
            &["clone", "-q", &self.bare.path().display().to_string(), "."],
        );
        write(clone.path(), file, content);
        run_git(clone.path(), &["add", "-A"]);
        run_git(
            clone.path(),
            &["commit", "-q", "-m", &format!("remote: {file}")],
        );
        run_git(clone.path(), &["push", "-q", "origin", "main"]);
        String::from_utf8_lossy(&run_git(clone.path(), &["rev-parse", "HEAD"]))
            .trim()
            .to_owned()
    }
    async fn pull(
        &self,
        strategy: PullStrategy,
        autostash: bool,
    ) -> Result<PullOutcome, PullError> {
        let head = self.head();
        self.git
            .pull(
                &self.info,
                &PullRequest {
                    strategy,
                    expected_head: &head,
                    autostash,
                },
                None,
                &Cancel::new(),
                |_| {},
            )
            .await
    }
}

async fn env(ssh: bool) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fakessh");
    std::fs::write(&script, include_str!("common/fake_ssh.sh")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.path().join("mode"), "ok").unwrap();
    let bare = tempfile::tempdir().unwrap();
    run_git(bare.path(), &["init", "-q", "--bare", "-b", "main"]);
    let seed = tempfile::tempdir().unwrap();
    run_git(seed.path(), &["init", "-q", "-b", "main"]);
    write(seed.path(), "a.txt", "one\n");
    commit_all(seed.path(), "first");
    run_git(
        seed.path(),
        &["push", "-q", &bare.path().display().to_string(), "main"],
    );
    let repo = tempfile::tempdir().unwrap();
    run_git(
        repo.path(),
        &["clone", "-q", &bare.path().display().to_string(), "."],
    );
    if ssh {
        let url = format!("ssh://git@fakehost{}", bare.path().display());
        run_git(repo.path(), &["remote", "set-url", "origin", &url]);
    } else {
        run_git(
            repo.path(),
            &[
                "remote",
                "set-url",
                "origin",
                &format!("file://{}", bare.path().display()),
            ],
        );
    }
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com")
        .with_env("GIT_SSH_COMMAND", script.as_os_str())
        .with_env("FAKESSH_DIR", dir.path().as_os_str());
    let info = git.open(repo.path(), &Cancel::new()).await.unwrap();
    Env {
        dir,
        bare,
        repo,
        git,
        info,
    }
}

#[tokio::test]
async fn the_preview_names_the_tracked_branch_and_the_state_before_integrating() {
    let e = env(false).await;
    let preview = e.git.pull_preview(&e.info).await.unwrap();
    assert_eq!(
        (
            preview.branch.as_str(),
            preview.upstream.as_str(),
            preview.remote.as_str()
        ),
        ("main", "origin/main", "origin")
    );
    assert_eq!(preview.kind, PullKind::UpToDate);
    write(e.path(), "mine.txt", "m\n");
    commit_all(e.path(), "mine");
    assert_eq!(
        e.git.pull_preview(&e.info).await.unwrap().kind,
        PullKind::Ahead
    );
    e.actor("theirs.txt", "t\n");
    e.git
        .fetch_remote(&e.info, "origin", &Cancel::new(), |_| {})
        .await
        .unwrap();
    let preview = e.git.pull_preview(&e.info).await.unwrap();
    assert_eq!(
        (preview.kind, preview.ahead, preview.behind),
        (PullKind::Diverged, 1, 1)
    );
}

#[tokio::test]
async fn a_branch_that_is_behind_fast_forwards_with_every_strategy() {
    for strategy in [
        PullStrategy::FastForwardOnly,
        PullStrategy::Merge,
        PullStrategy::Rebase,
    ] {
        let e = env(false).await;
        let theirs = e.actor("theirs.txt", "t\n");
        let outcome = e.pull(strategy, false).await.unwrap();
        assert_eq!(outcome.fetched_updates, 1);
        assert!(
            matches!(
                outcome.integration,
                Integration::FastForward { .. } | Integration::Rebased { .. }
            ),
            "{strategy:?}: {outcome:?}"
        );
        assert_eq!(e.head(), theirs, "{strategy:?}");
        assert!(e.path().join("theirs.txt").exists());
    }
}

#[tokio::test]
async fn a_divergence_is_not_hidden_ff_only_stops_merge_and_rebase_integrate() {
    let diverge = |e: &Env| {
        e.actor("theirs.txt", "t\n");
        write(e.path(), "mine.txt", "m\n");
        commit_all(e.path(), "mine");
    };
    let ff = env(false).await;
    diverge(&ff);
    let head = ff.head();
    assert_eq!(
        ff.pull(PullStrategy::FastForwardOnly, false)
            .await
            .unwrap_err(),
        PullError::NotFastForward
    );
    assert_eq!(ff.head(), head);
    assert_eq!(ff.out(&["rev-parse", "refs/remotes/origin/main"]).len(), 40);
    let merged = env(false).await;
    diverge(&merged);
    let head = merged.head();
    let outcome = merged.pull(PullStrategy::Merge, false).await.unwrap();
    assert!(
        matches!(outcome.integration, Integration::Merged { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        merged
            .out(&["rev-list", "--parents", "-n", "1", "HEAD"])
            .split(' ')
            .count(),
        3
    );
    assert_eq!(merged.out(&["rev-parse", "HEAD^1"]), head);
    assert!(
        merged
            .out(&["log", "-1", "--format=%s"])
            .starts_with("Merge remote-tracking branch 'origin/main'")
    );
    let rebased = env(false).await;
    diverge(&rebased);
    let old = rebased.head();
    let outcome = rebased.pull(PullStrategy::Rebase, false).await.unwrap();
    let Integration::Rebased { marker, .. } = outcome.integration else {
        panic!("{outcome:?}");
    };
    assert_eq!(
        rebased.out(&["rev-list", "--merges", "--count", "HEAD"]),
        "0"
    );
    assert_eq!(rebased.out(&["log", "-1", "--format=%s"]), "mine");
    assert_eq!(
        rebased.out(&["log", "-2", "--format=%s"]).lines().nth(1),
        Some("remote: theirs.txt")
    );
    assert_eq!(rebased.out(&["rev-parse", &marker]), old);
}

#[tokio::test]
async fn an_integration_conflict_joins_the_resolution_path_and_can_be_abandoned() {
    for strategy in [PullStrategy::Merge, PullStrategy::Rebase] {
        let e = env(false).await;
        e.actor("a.txt", "theirs\n");
        write(e.path(), "a.txt", "mine\n");
        commit_all(e.path(), "mine");
        let head = e.head();
        let outcome = e.pull(strategy, false).await.unwrap();
        assert_eq!(
            outcome.integration,
            Integration::Conflicts {
                files: vec![PathBuf::from("a.txt")]
            },
            "{strategy:?}"
        );
        assert!(e.git.operation_state(&e.info).await.unwrap().is_some());
        e.git
            .abort_operation(&e.info, &[PathBuf::from("a.txt")])
            .await
            .unwrap();
        assert_eq!(e.head(), head, "{strategy:?}");
    }
}

#[tokio::test]
async fn a_network_failure_or_a_cancellation_leaves_the_local_work_and_refs_exactly_as_they_were() {
    let e = env(true).await;
    write(e.path(), "a.txt", "unsaved edit\n");
    write(e.path(), "new.txt", "untracked\n");
    let snapshot = |e: &Env| {
        (
            e.head(),
            e.out(&["status", "--porcelain=v2"]),
            e.out(&["for-each-ref", "refs/remotes"]),
        )
    };
    let before = snapshot(&e);
    std::fs::write(e.dir.path().join("mode"), "offline").unwrap();
    let error = e.pull(PullStrategy::Merge, false).await.unwrap_err();
    assert!(
        matches!(error, PullError::Fetch(NetError::Network(_))),
        "{error:?}"
    );
    assert_eq!(snapshot(&e), before);
    std::fs::write(e.dir.path().join("mode"), "hang").unwrap();
    let cancel = Cancel::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        canceller.cancel();
    });
    let head = e.head();
    let error = e
        .git
        .pull(
            &e.info,
            &PullRequest {
                strategy: PullStrategy::Rebase,
                expected_head: &head,
                autostash: false,
            },
            None,
            &cancel,
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(error, PullError::Fetch(NetError::Cancelled));
    assert_eq!(snapshot(&e), before);
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "unsaved edit\n"
    );
    std::fs::write(e.dir.path().join("mode"), "ok").unwrap();
    e.actor("theirs.txt", "t\n");
    write(e.path(), "a.txt", "one\n");
    e.pull(PullStrategy::Merge, false).await.unwrap();
}

#[tokio::test]
async fn overlapping_local_work_blocks_the_integration_unless_it_is_put_aside() {
    let e = env(false).await;
    e.actor("a.txt", "theirs\n");
    write(e.path(), "a.txt", "my unsaved work\n");
    let head = e.head();
    assert_eq!(
        e.pull(PullStrategy::FastForwardOnly, false)
            .await
            .unwrap_err(),
        PullError::LocalChanges
    );
    assert_eq!(e.head(), head);
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "my unsaved work\n"
    );
    let outcome = e.pull(PullStrategy::Merge, true).await.unwrap();
    assert_eq!(
        outcome.integration,
        Integration::Conflicts {
            files: vec![PathBuf::from("a.txt")]
        },
        "putting the work back conflicts: it is surfaced, not hidden"
    );
    assert_eq!(
        e.out(&["stash", "list"]).lines().count(),
        1,
        "the work stays in the stash after a conflicting reapplication"
    );
    let op = e.git.operation_state(&e.info).await.unwrap().unwrap();
    assert_eq!(op.kind, gitpoulpe_core::OperationKind::StashApply);
}

#[tokio::test]
async fn nothing_to_integrate_is_reported_as_up_to_date() {
    let e = env(false).await;
    let outcome = e.pull(PullStrategy::Rebase, false).await.unwrap();
    assert_eq!(
        outcome,
        PullOutcome {
            fetched_updates: 0,
            integration: Integration::UpToDate
        }
    );
    write(e.path(), "mine.txt", "m\n");
    commit_all(e.path(), "mine");
    let outcome = e.pull(PullStrategy::Merge, false).await.unwrap();
    assert_eq!(outcome.integration, Integration::UpToDate);
}

#[tokio::test]
async fn bad_situations_are_refused_without_touching_anything() {
    let e = env(false).await;
    let head = e.head();
    assert_eq!(
        e.git
            .pull(
                &e.info,
                &PullRequest {
                    strategy: PullStrategy::Merge,
                    expected_head: &"0".repeat(40),
                    autostash: false
                },
                None,
                &Cancel::new(),
                |_| {}
            )
            .await
            .unwrap_err(),
        PullError::HeadMoved
    );
    run_git(e.path(), &["config", "--unset", "branch.main.remote"]);
    run_git(e.path(), &["config", "--unset", "branch.main.merge"]);
    assert_eq!(
        e.pull(PullStrategy::Merge, false).await.unwrap_err(),
        PullError::NoUpstream
    );
    run_git(e.path(), &["checkout", "-q", "--detach"]);
    assert_eq!(
        e.pull(PullStrategy::Merge, false).await.unwrap_err(),
        PullError::NoBranch
    );
    assert_eq!(e.head(), head);
}

#[tokio::test]
async fn another_operation_in_progress_blocks_the_pull_before_any_network_access() {
    let e = env(false).await;
    run_git(e.path(), &["checkout", "-q", "-b", "side"]);
    write(e.path(), "a.txt", "side\n");
    commit_all(e.path(), "side");
    run_git(e.path(), &["checkout", "-q", "main"]);
    write(e.path(), "a.txt", "main\n");
    commit_all(e.path(), "main");
    assert!(
        !common::git_output(e.path(), &["merge", "side"])
            .status
            .success()
    );
    e.actor("theirs.txt", "t\n");
    let before = e.out(&["for-each-ref", "refs/remotes"]);
    assert_eq!(
        e.pull(PullStrategy::Merge, false).await.unwrap_err(),
        PullError::OperationInProgress
    );
    assert_eq!(
        e.out(&["for-each-ref", "refs/remotes"]),
        before,
        "nothing was fetched"
    );
}
