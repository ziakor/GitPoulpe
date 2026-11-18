mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use common::{commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::{PushKind, PushMode, SessionInfo};
use gitpoulpe_git::{Cancel, Git, NetError, PushError, PushRejection, PushRequest};

struct Env {
    dir: tempfile::TempDir,
    bare: tempfile::TempDir,
    repo: tempfile::TempDir,
    git: Git,
    info: SessionInfo,
}

impl Env {
    fn out(&self, dir: &Path, args: &[&str]) -> String {
        String::from_utf8_lossy(&run_git(dir, args))
            .trim()
            .to_owned()
    }
    fn local(&self, args: &[&str]) -> String {
        self.out(self.repo.path(), args)
    }
    fn remote_tip(&self, branch: &str) -> String {
        self.out(
            self.bare.path(),
            &["rev-parse", &format!("refs/heads/{branch}")],
        )
    }
    fn other_actor(&self, name: &str, content: &str) -> String {
        let clone = tempfile::tempdir().unwrap();
        run_git(
            clone.path(),
            &["clone", "-q", &self.bare.path().display().to_string(), "."],
        );
        write(clone.path(), name, content);
        run_git(clone.path(), &["add", "-A"]);
        run_git(
            clone.path(),
            &["commit", "-q", "-m", &format!("other: {name}")],
        );
        run_git(clone.path(), &["push", "-q", "origin", "main"]);
        self.out(clone.path(), &["rev-parse", "HEAD"])
    }
    async fn fetch(&self) {
        self.git
            .fetch_remote(&self.info, "origin", &Cancel::new(), |_| {})
            .await
            .unwrap();
    }
    async fn push(
        &self,
        mode: PushMode,
        expected: Option<&str>,
        upstream: bool,
    ) -> Result<gitpoulpe_core::PushOutcome, PushError> {
        self.git
            .push_branch(
                &self.info,
                &PushRequest {
                    branch: "main",
                    remote: "origin",
                    mode,
                    expected_remote_oid: expected,
                    set_upstream: upstream,
                },
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
    let repo = init_repo();
    write(repo.path(), "a.txt", "one\n");
    commit_all(repo.path(), "first");
    let url = if ssh {
        format!("ssh://git@fakehost{}", bare.path().display())
    } else {
        format!("file://{}", bare.path().display())
    };
    run_git(repo.path(), &["remote", "add", "origin", &url]);
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
async fn publishing_a_new_branch_names_the_destination_sets_the_upstream_and_is_verified() {
    let e = env(false).await;
    let preview = e.git.push_preview(&e.info, "main", "origin").await.unwrap();
    assert_eq!(preview.kind, PushKind::NewBranch);
    assert_eq!(preview.destination, "refs/heads/main");
    assert_eq!(preview.upstream, None);
    let outcome = e.push(PushMode::Normal, None, true).await.unwrap();
    assert!(outcome.verified && outcome.set_upstream);
    assert_eq!(e.remote_tip("main"), e.local(&["rev-parse", "HEAD"]));
    assert_eq!(e.local(&["config", "branch.main.remote"]), "origin");
    assert_eq!(e.local(&["config", "branch.main.merge"]), "refs/heads/main");
    assert_eq!(
        e.local(&["rev-parse", "refs/remotes/origin/main"]),
        e.remote_tip("main")
    );
    let after = e.git.push_preview(&e.info, "main", "origin").await.unwrap();
    assert_eq!(after.kind, PushKind::UpToDate);
    assert_eq!(after.upstream.as_deref(), Some("origin/main"));
}

#[tokio::test]
async fn new_local_commits_fast_forward_the_remote() {
    let e = env(false).await;
    e.push(PushMode::Normal, None, true).await.unwrap();
    write(e.repo.path(), "b.txt", "two\n");
    commit_all(e.repo.path(), "second");
    let preview = e.git.push_preview(&e.info, "main", "origin").await.unwrap();
    assert_eq!(
        (preview.kind, preview.ahead, preview.behind),
        (PushKind::FastForward, 1, 0)
    );
    let outcome = e.push(PushMode::Normal, None, false).await.unwrap();
    assert!(outcome.verified && !outcome.up_to_date);
    assert_eq!(e.remote_tip("main"), e.local(&["rev-parse", "HEAD"]));
    let again = e.push(PushMode::Normal, None, false).await.unwrap();
    assert!(again.up_to_date);
}

#[tokio::test]
async fn a_remote_that_moved_refuses_the_push_and_nothing_is_lost() {
    let e = env(false).await;
    e.push(PushMode::Normal, None, true).await.unwrap();
    let theirs = e.other_actor("theirs.txt", "t\n");
    write(e.repo.path(), "mine.txt", "m\n");
    commit_all(e.repo.path(), "mine");
    let local = e.local(&["rev-parse", "HEAD"]);
    let error = e.push(PushMode::Normal, None, false).await.unwrap_err();
    assert!(
        matches!(
            error,
            PushError::Rejected(PushRejection::FetchFirst | PushRejection::NonFastForward)
        ),
        "{error:?}"
    );
    assert_eq!(e.remote_tip("main"), theirs);
    assert_eq!(e.local(&["rev-parse", "HEAD"]), local);
    e.fetch().await;
    let preview = e.git.push_preview(&e.info, "main", "origin").await.unwrap();
    assert_eq!(
        (preview.kind, preview.ahead, preview.behind),
        (PushKind::Diverged, 1, 1)
    );
    let error = e.push(PushMode::Normal, None, false).await.unwrap_err();
    assert_eq!(error, PushError::Rejected(PushRejection::NonFastForward));
    assert_eq!(e.remote_tip("main"), theirs);
}

#[tokio::test]
async fn a_hook_refusal_is_reported_as_the_remotes_decision() {
    let e = env(false).await;
    let hook = e.bare.path().join("hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\necho 'no pushes today' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let error = e.push(PushMode::Normal, None, false).await.unwrap_err();
    assert!(
        matches!(error, PushError::Rejected(PushRejection::Hook(_))),
        "{error:?}"
    );
    assert!(
        e.out(e.bare.path(), &["for-each-ref", "refs/heads"])
            .is_empty()
    );
    assert_eq!(e.local(&["log", "-1", "--format=%s"]), "first");
}

#[tokio::test]
async fn a_rewritten_history_is_published_only_against_the_expected_remote_state() {
    let e = env(false).await;
    e.push(PushMode::Normal, None, true).await.unwrap();
    let expected = e.local(&["rev-parse", "refs/remotes/origin/main"]);
    run_git(
        e.repo.path(),
        &["commit", "-q", "--amend", "-m", "first, reworded"],
    );
    let rewritten = e.local(&["rev-parse", "HEAD"]);
    let preview = e.git.push_preview(&e.info, "main", "origin").await.unwrap();
    assert_eq!(preview.kind, PushKind::Diverged);
    assert_eq!(preview.would_discard.len(), 1);
    assert_eq!(preview.would_discard[0].oid, expected);
    assert_eq!(
        e.push(PushMode::Normal, None, false).await.unwrap_err(),
        PushError::Rejected(PushRejection::NonFastForward)
    );
    let outcome = e
        .push(PushMode::ForceWithLease, Some(&expected), false)
        .await
        .unwrap();
    assert!(outcome.verified);
    assert_eq!(e.remote_tip("main"), rewritten);
}

#[tokio::test]
async fn a_concurrent_push_after_the_expected_state_is_never_overwritten() {
    let e = env(false).await;
    e.push(PushMode::Normal, None, true).await.unwrap();
    let expected = e.local(&["rev-parse", "refs/remotes/origin/main"]);
    run_git(
        e.repo.path(),
        &["commit", "-q", "--amend", "-m", "rewritten"],
    );
    let theirs = e.other_actor("theirs.txt", "someone else\n");
    let error = e
        .push(PushMode::ForceWithLease, Some(&expected), false)
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            PushError::Rejected(PushRejection::StaleLease | PushRejection::FetchFirst)
        ),
        "{error:?}"
    );
    assert_eq!(e.remote_tip("main"), theirs);
    e.fetch().await;
    let preview = e.git.push_preview(&e.info, "main", "origin").await.unwrap();
    assert_eq!(preview.would_discard.len(), 2);
    assert!(
        preview
            .would_discard
            .iter()
            .any(|c| c.subject == "other: theirs.txt")
    );
    let fresh = e.local(&["rev-parse", "refs/remotes/origin/main"]);
    e.push(PushMode::ForceWithLease, Some(&fresh), false)
        .await
        .unwrap();
    assert_eq!(e.remote_tip("main"), e.local(&["rev-parse", "HEAD"]));
}

#[tokio::test]
async fn the_protected_force_needs_a_real_expected_state_and_a_plain_force_does_not_exist() {
    let e = env(false).await;
    e.push(PushMode::Normal, None, true).await.unwrap();
    run_git(
        e.repo.path(),
        &["commit", "-q", "--amend", "-m", "rewritten"],
    );
    let before = e.remote_tip("main");
    for bad in [None, Some("main"), Some("--force"), Some("abc")] {
        assert_eq!(
            e.push(PushMode::ForceWithLease, bad, false)
                .await
                .unwrap_err(),
            PushError::NothingKnown,
            "{bad:?}"
        );
    }
    assert_eq!(e.remote_tip("main"), before);
}

#[tokio::test]
async fn bad_branch_and_remote_names_are_refused() {
    let e = env(false).await;
    for (branch, remote, expected) in [
        ("ghost", "origin", PushError::InvalidBranch),
        ("-x", "origin", PushError::InvalidBranch),
        ("main", "ghost", PushError::NoSuchRemote),
        ("main", "--all", PushError::NoSuchRemote),
    ] {
        let error = e
            .git
            .push_branch(
                &e.info,
                &PushRequest {
                    branch,
                    remote,
                    mode: PushMode::Normal,
                    expected_remote_oid: None,
                    set_upstream: false,
                },
                &Cancel::new(),
                |_| {},
            )
            .await
            .unwrap_err();
        assert_eq!(error, expected, "{branch} {remote}");
    }
    assert!(
        e.out(e.bare.path(), &["for-each-ref", "refs/heads"])
            .is_empty()
    );
}

#[tokio::test]
async fn publishing_works_over_ssh_and_a_cancellation_leaves_the_remote_untouched() {
    let e = env(true).await;
    std::fs::write(e.dir.path().join("mode"), "hang").unwrap();
    let cancel = Cancel::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(700)).await;
        canceller.cancel();
    });
    let error = e
        .git
        .push_branch(
            &e.info,
            &PushRequest {
                branch: "main",
                remote: "origin",
                mode: PushMode::Normal,
                expected_remote_oid: None,
                set_upstream: true,
            },
            &cancel,
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(error, PushError::Net(NetError::Cancelled));
    assert!(
        e.out(e.bare.path(), &["for-each-ref", "refs/heads"])
            .is_empty()
    );
    assert!(
        e.git
            .push_preview(&e.info, "main", "origin")
            .await
            .unwrap()
            .upstream
            .is_none()
    );
    std::fs::write(e.dir.path().join("mode"), "ok").unwrap();
    let outcome = e.push(PushMode::Normal, None, true).await.unwrap();
    assert!(outcome.verified);
    assert_eq!(e.remote_tip("main"), e.local(&["rev-parse", "HEAD"]));
}

#[tokio::test]
async fn the_expected_state_is_the_one_captured_before_the_rewrite_not_whatever_was_fetched_since()
{
    let e = env(false).await;
    e.push(PushMode::Normal, None, true).await.unwrap();
    let captured = e.local(&["rev-parse", "refs/remotes/origin/main"]);
    run_git(
        e.repo.path(),
        &["commit", "-q", "--amend", "-m", "rewritten"],
    );
    let theirs = e.other_actor("theirs.txt", "t\n");
    e.fetch().await;
    assert_eq!(e.local(&["rev-parse", "refs/remotes/origin/main"]), theirs);
    let error = e
        .push(PushMode::ForceWithLease, Some(&captured), false)
        .await
        .unwrap_err();
    assert_eq!(error, PushError::Rejected(PushRejection::StaleLease));
    assert_eq!(e.remote_tip("main"), theirs);
}

#[tokio::test]
async fn a_remote_that_does_not_keep_the_pushed_value_is_reported_as_unverified() {
    let e = env(false).await;
    e.push(PushMode::Normal, None, true).await.unwrap();
    let first = e.local(&["rev-parse", "HEAD"]);
    write(e.repo.path(), "b.txt", "two\n");
    commit_all(e.repo.path(), "second");
    let hook = e.bare.path().join("hooks/post-receive");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\ngit update-ref refs/heads/main {first}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let outcome = e.push(PushMode::Normal, None, false).await.unwrap();
    assert!(!outcome.verified);
    assert_eq!(e.remote_tip("main"), first);
}

#[tokio::test]
async fn the_destination_is_always_the_explicit_branch_reference() {
    let e = env(false).await;
    run_git(e.repo.path(), &["tag", "main"]);
    let outcome = e.push(PushMode::Normal, None, false).await.unwrap();
    assert!(outcome.verified);
    assert_eq!(outcome.destination, "refs/heads/main");
    assert!(
        e.out(e.bare.path(), &["for-each-ref", "refs/tags"])
            .is_empty()
    );
}
