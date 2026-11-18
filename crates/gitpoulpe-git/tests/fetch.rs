mod common;

use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{Cancel, Git, NetError};

struct Env {
    dir: tempfile::TempDir,
    source: tempfile::TempDir,
    repo: tempfile::TempDir,
    git: Git,
    info: SessionInfo,
}

impl Env {
    fn out(&self, dir: &std::path::Path, args: &[&str]) -> String {
        String::from_utf8_lossy(&run_git(dir, args))
            .trim()
            .to_owned()
    }
    fn local(&self, args: &[&str]) -> String {
        self.out(self.repo.path(), args)
    }
    fn remote_refs(&self) -> String {
        self.local(&[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/remotes",
        ])
    }
}

async fn env(ssh: bool) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fakessh");
    std::fs::write(&script, include_str!("common/fake_ssh.sh")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.path().join("mode"), "ok").unwrap();
    let source = init_repo();
    write(source.path(), "a.txt", "one\n");
    commit_all(source.path(), "first");
    let repo = init_repo();
    write(repo.path(), "a.txt", "one\n");
    write(repo.path(), "keep.txt", "keep\n");
    commit_all(repo.path(), "local");
    let url = if ssh {
        format!("ssh://git@fakehost{}", source.path().display())
    } else {
        format!("file://{}", source.path().display())
    };
    run_git(repo.path(), &["remote", "add", "origin", &url]);
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_SSH_COMMAND", script.as_os_str())
        .with_env("FAKESSH_DIR", dir.path().as_os_str());
    let info = git.open(repo.path(), &Cancel::new()).await.unwrap();
    Env {
        dir,
        source,
        repo,
        git,
        info,
    }
}

#[tokio::test]
async fn new_branches_and_commits_become_remote_references() {
    let e = env(false).await;
    run_git(e.source.path(), &["branch", "dev"]);
    write(e.source.path(), "b.txt", "two\n");
    commit_all(e.source.path(), "second");
    run_git(e.source.path(), &["tag", "v1"]);
    let outcome = e
        .git
        .fetch_remote(&e.info, "origin", &Cancel::new(), |_| {})
        .await
        .unwrap();
    let main = e.out(e.source.path(), &["rev-parse", "main"]);
    let dev = e.out(e.source.path(), &["rev-parse", "dev"]);
    assert_eq!(e.local(&["rev-parse", "refs/remotes/origin/main"]), main);
    assert_eq!(e.local(&["rev-parse", "refs/remotes/origin/dev"]), dev);
    assert_eq!(e.local(&["rev-parse", "refs/tags/v1"]), main);
    assert!(
        outcome
            .updates
            .iter()
            .any(|u| u.name == "refs/remotes/origin/dev" && u.is_new())
    );
    let again = e
        .git
        .fetch_remote(&e.info, "origin", &Cancel::new(), |_| {})
        .await
        .unwrap();
    assert!(again.updates.is_empty());
    write(e.source.path(), "c.txt", "three\n");
    commit_all(e.source.path(), "third");
    let third = e
        .git
        .fetch_remote(&e.info, "origin", &Cancel::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(third.updates.len(), 1);
    assert!(!third.updates[0].is_new());
    assert_eq!(
        third.updates[0].new,
        e.out(e.source.path(), &["rev-parse", "main"])
    );
}

#[tokio::test]
async fn a_fetch_leaves_the_index_the_working_tree_and_local_branches_untouched() {
    let e = env(false).await;
    run_git(e.repo.path(), &["branch", "side"]);
    write(e.repo.path(), "keep.txt", "changed in the work tree\n");
    write(e.repo.path(), "staged.txt", "staged\n");
    run_git(e.repo.path(), &["add", "staged.txt"]);
    write(e.repo.path(), "untracked.txt", "u\n");
    write(e.source.path(), "b.txt", "two\n");
    commit_all(e.source.path(), "second");
    let snapshot = |e: &Env| {
        (
            e.local(&["status", "--porcelain=v2", "--branch"]),
            e.local(&["ls-files", "--stage"]),
            e.local(&["diff"]),
            e.local(&["diff", "--cached"]),
            e.local(&["for-each-ref", "refs/heads"]),
            std::fs::read_to_string(e.repo.path().join("keep.txt")).unwrap(),
        )
    };
    let before = snapshot(&e);
    e.git
        .fetch_remote(&e.info, "origin", &Cancel::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(snapshot(&e), before);
    assert!(!e.remote_refs().is_empty());
}

#[tokio::test]
async fn an_interrupted_fetch_keeps_the_references_consistent_and_can_be_relaunched() {
    let e = env(true).await;
    write(e.source.path(), "b.txt", "two\n");
    commit_all(e.source.path(), "second");
    std::fs::write(e.dir.path().join("mode"), "hang").unwrap();
    let before = e.remote_refs();
    let cancel = Cancel::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        canceller.cancel();
    });
    let error = e
        .git
        .fetch_remote(&e.info, "origin", &cancel, |_| {})
        .await
        .unwrap_err();
    assert_eq!(error, NetError::Cancelled);
    assert_eq!(e.remote_refs(), before);
    run_git(e.repo.path(), &["fsck", "--strict"]);
    std::fs::write(e.dir.path().join("mode"), "ok").unwrap();
    e.git
        .fetch_remote(&e.info, "origin", &Cancel::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(
        e.local(&["rev-parse", "refs/remotes/origin/main"]),
        e.out(e.source.path(), &["rev-parse", "main"])
    );
    run_git(e.repo.path(), &["fsck", "--strict"]);
}

#[tokio::test]
async fn a_failed_fetch_changes_nothing_and_a_later_one_succeeds() {
    let e = env(true).await;
    std::fs::write(e.dir.path().join("mode"), "offline").unwrap();
    let before = e.remote_refs();
    let error = e
        .git
        .fetch_remote(&e.info, "origin", &Cancel::new(), |_| {})
        .await
        .unwrap_err();
    assert!(matches!(error, NetError::Network(_)), "{error:?}");
    assert_eq!(e.remote_refs(), before);
    std::fs::write(e.dir.path().join("mode"), "ok").unwrap();
    e.git
        .fetch_remote(&e.info, "origin", &Cancel::new(), |_| {})
        .await
        .unwrap();
    assert!(!e.remote_refs().is_empty());
}

#[tokio::test]
async fn only_an_existing_remote_can_be_fetched() {
    let e = env(false).await;
    for name in ["ghost", "-x", "--all"] {
        let error = e
            .git
            .fetch_remote(&e.info, name, &Cancel::new(), |_| {})
            .await
            .unwrap_err();
        assert!(matches!(error, NetError::Refused(_)), "{name}: {error:?}");
    }
}

#[tokio::test]
async fn progress_is_reported_for_a_real_transfer() {
    let e = env(false).await;
    for i in 0..60 {
        write(e.source.path(), &format!("f{i}.txt"), &format!("{i}\n"));
        commit_all(e.source.path(), &format!("c{i}"));
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    e.git
        .fetch_remote(&e.info, "origin", &Cancel::new(), move |p| {
            sink.lock().unwrap().push(p);
        })
        .await
        .unwrap();
    assert!(!seen.lock().unwrap().is_empty());
}
