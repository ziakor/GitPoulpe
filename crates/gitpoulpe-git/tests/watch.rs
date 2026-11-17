mod common;

use std::path::Path;
use std::time::Duration;

use common::*;
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{Cancel, Git, RepoWatcher, Signal, WatchError, coalesce, is_relevant};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

const QUIET: Duration = Duration::from_millis(120);
const LONG: Duration = Duration::from_millis(600);

async fn session(path: &Path) -> SessionInfo {
    Git::new().open(path, &Cancel::new()).await.unwrap()
}

fn start(info: &SessionInfo) -> (RepoWatcher, UnboundedReceiver<Signal>) {
    let (raw_tx, raw_rx) = unbounded_channel();
    let (out_tx, out_rx) = unbounded_channel();
    let watcher = RepoWatcher::start(info, raw_tx).expect("the watcher starts");
    tokio::spawn(coalesce(raw_rx, out_tx, QUIET, LONG));
    (watcher, out_rx)
}

async fn next(rx: &mut UnboundedReceiver<Signal>, within: Duration) -> Option<Signal> {
    tokio::time::timeout(within, rx.recv()).await.ok().flatten()
}

async fn drain(rx: &mut UnboundedReceiver<Signal>, quiet_for: Duration) -> usize {
    let mut count = 0;
    while next(rx, quiet_for).await.is_some() {
        count += 1;
    }
    count
}

fn committed_repo() -> tempfile::TempDir {
    let repo = init_repo();
    write(repo.path(), "a.txt", "a\n");
    commit_all(repo.path(), "base");
    repo
}

#[tokio::test]
async fn editing_a_file_in_the_working_directory_is_reported_once_it_settles() {
    let repo = committed_repo();
    let (_watcher, mut rx) = start(&session(repo.path()).await);
    tokio::time::sleep(Duration::from_millis(200)).await;
    write(repo.path(), "a.txt", "changed\n");
    assert_eq!(
        next(&mut rx, Duration::from_secs(5)).await,
        Some(Signal::Changed)
    );
}

#[tokio::test]
async fn a_burst_of_writes_becomes_a_handful_of_signals_not_one_per_event() {
    let repo = committed_repo();
    let (_watcher, mut rx) = start(&session(repo.path()).await);
    tokio::time::sleep(Duration::from_millis(200)).await;
    for i in 0..300 {
        write(repo.path(), &format!("burst/file{i}.txt"), "x\n");
    }
    assert!(next(&mut rx, Duration::from_secs(5)).await.is_some());
    let extra = drain(&mut rx, Duration::from_millis(700)).await;
    assert!(
        extra <= 2,
        "300 writes produced {} signals in total",
        extra + 1
    );
}

#[tokio::test]
async fn staging_and_committing_are_noticed_through_the_index_and_references() {
    let repo = committed_repo();
    let (_watcher, mut rx) = start(&session(repo.path()).await);
    tokio::time::sleep(Duration::from_millis(200)).await;
    write(repo.path(), "b.txt", "b\n");
    drain(&mut rx, Duration::from_millis(600)).await;
    git(repo.path(), &["add", "b.txt"]);
    assert!(
        next(&mut rx, Duration::from_secs(5)).await.is_some(),
        "git add changes the index"
    );
    drain(&mut rx, Duration::from_millis(600)).await;
    git(repo.path(), &["commit", "-q", "-m", "second"]);
    assert!(
        next(&mut rx, Duration::from_secs(5)).await.is_some(),
        "a commit moves HEAD and the branch reference"
    );
}

#[tokio::test]
async fn object_database_writes_and_lock_files_are_ignored() {
    let repo = committed_repo();
    let info = session(repo.path()).await;
    let (_watcher, mut rx) = start(&info);
    tokio::time::sleep(Duration::from_millis(200)).await;
    std::fs::write(info.git_dir.join("objects").join("noise"), "x").unwrap();
    std::fs::write(info.git_dir.join("index.lock"), "x").unwrap();
    std::fs::write(info.git_dir.join("COMMIT_EDITMSG"), "x").unwrap();
    assert_eq!(
        drain(&mut rx, Duration::from_millis(1200)).await,
        0,
        "none of these changes what Git reports"
    );
}

#[tokio::test]
async fn reading_the_repository_with_git_never_triggers_the_watcher() {
    let repo = committed_repo();
    write(repo.path(), "a.txt", "dirty\n");
    let git_cli = Git::new();
    let info = session(repo.path()).await;
    let (_watcher, mut rx) = start(&info);
    tokio::time::sleep(Duration::from_millis(200)).await;
    for generation in 1..=5u32 {
        let mut g = gitpoulpe_core::Generation::default();
        for _ in 0..generation {
            g = g.next();
        }
        git_cli.read_status(&info, g, &Cancel::new()).await.unwrap();
    }
    assert_eq!(
        drain(&mut rx, Duration::from_millis(1000)).await,
        0,
        "our own reads must not cause a refresh loop"
    );
}

#[tokio::test]
async fn a_linked_worktree_hears_about_commits_made_in_the_main_repository() {
    let repo = committed_repo();
    let holder = tempfile::tempdir().unwrap();
    let linked = holder.path().join("linked");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            linked.to_str().unwrap(),
        ],
    );
    let info = session(&linked).await;
    assert_ne!(info.git_dir, info.common_dir);
    let (_watcher, mut rx) = start(&info);
    tokio::time::sleep(Duration::from_millis(200)).await;
    write(&linked, "only-here.txt", "x\n");
    assert!(
        next(&mut rx, Duration::from_secs(5)).await.is_some(),
        "an edit in the linked worktree"
    );
    drain(&mut rx, Duration::from_millis(600)).await;
    git(&linked, &["add", "only-here.txt"]);
    assert!(
        next(&mut rx, Duration::from_secs(5)).await.is_some(),
        "the linked worktree has its own index"
    );
}

#[tokio::test]
async fn a_watcher_that_cannot_start_reports_why_instead_of_staying_silent() {
    let repo = committed_repo();
    let mut info = session(repo.path()).await;
    info.worktree = Some(repo.path().join("vanished"));
    let (raw_tx, _raw_rx) = unbounded_channel();
    let error: WatchError = RepoWatcher::start(&info, raw_tx)
        .err()
        .expect("watching a missing directory fails");
    assert!(error.0.contains("vanished"), "{error}");
}

#[test]
fn only_paths_that_change_what_git_reports_are_relevant() {
    let info = SessionInfo {
        id: gitpoulpe_core::SessionId::next(),
        repo: gitpoulpe_core::RepoId::from_common_dir(Path::new("/nonexistent/w/.git")),
        kind: gitpoulpe_core::RepoKind::Ordinary,
        worktree: Some("/nonexistent/w".into()),
        git_dir: "/nonexistent/w/.git".into(),
        common_dir: "/nonexistent/w/.git".into(),
        has_commits: true,
    };
    for relevant in [
        "/nonexistent/w/src/a.rs",
        "/nonexistent/w/.git/index",
        "/nonexistent/w/.git/HEAD",
        "/nonexistent/w/.git/refs/heads/main",
        "/nonexistent/w/.git/packed-refs",
    ] {
        assert!(is_relevant(Path::new(relevant), &info), "{relevant}");
    }
    for ignored in [
        "/nonexistent/w/.git/objects/ab/cdef",
        "/nonexistent/w/.git/index.lock",
        "/nonexistent/w/.git/refs/heads/main.lock",
        "/nonexistent/w/.git/logs/HEAD",
        "/elsewhere/file",
    ] {
        assert!(!is_relevant(Path::new(ignored), &info), "{ignored}");
    }
}

fn ignoring_repo() -> tempfile::TempDir {
    let repo = init_repo();
    write(repo.path(), ".gitignore", "node_modules/\n*.log\ntarget/\n");
    write(repo.path(), "a.txt", "a\n");
    commit_all(repo.path(), "base");
    repo
}

#[tokio::test]
async fn ignored_directories_are_not_watched_and_ignored_files_stay_silent() {
    let repo = ignoring_repo();
    for i in 0..200 {
        write(
            repo.path(),
            &format!("node_modules/pkg{i}/deep/index.js"),
            "x\n",
        );
    }
    std::fs::create_dir_all(repo.path().join("src/inner")).unwrap();
    let (watcher, mut rx) = start(&session(repo.path()).await);
    assert!(
        watcher.watched_directories() < 20,
        "only the directories git cares about: {}",
        watcher.watched_directories()
    );
    tokio::time::sleep(Duration::from_millis(200)).await;

    write(repo.path(), "debug.log", "noise\n");
    write(repo.path(), "node_modules/pkg1/deep/index.js", "changed\n");
    assert_eq!(drain(&mut rx, Duration::from_millis(900)).await, 0);

    write(repo.path(), "src/inner/real.rs", "fn main() {}\n");
    assert_eq!(
        next(&mut rx, Duration::from_secs(5)).await,
        Some(Signal::Changed)
    );
}

#[tokio::test]
async fn a_directory_created_after_start_is_watched_unless_git_ignores_it() {
    let repo = ignoring_repo();
    let (_watcher, mut rx) = start(&session(repo.path()).await);
    tokio::time::sleep(Duration::from_millis(200)).await;

    write(repo.path(), "feature/sub/one.rs", "1\n");
    assert!(next(&mut rx, Duration::from_secs(5)).await.is_some());
    drain(&mut rx, Duration::from_millis(700)).await;

    write(repo.path(), "feature/sub/two.rs", "2\n");
    assert!(
        next(&mut rx, Duration::from_secs(5)).await.is_some(),
        "the new directory is watched"
    );
    drain(&mut rx, Duration::from_millis(700)).await;

    write(repo.path(), "target/debug/out.o", "x\n");
    write(repo.path(), "target/debug/more.o", "y\n");
    assert_eq!(
        drain(&mut rx, Duration::from_millis(900)).await,
        0,
        "a new ignored directory stays silent"
    );
}

#[tokio::test]
async fn a_nested_repository_is_left_alone() {
    let repo = ignoring_repo();
    let nested = repo.path().join("vendor/inner");
    std::fs::create_dir_all(&nested).unwrap();
    git(&nested, &["init", "-q", "-b", "main"]);
    let (watcher, mut rx) = start(&session(repo.path()).await);
    tokio::time::sleep(Duration::from_millis(200)).await;
    write(&nested, "inside.txt", "x\n");
    assert_eq!(drain(&mut rx, Duration::from_millis(900)).await, 0);
    assert!(watcher.watched_directories() < 10);
}

#[tokio::test]
#[ignore = "measures a real repository: GITPOULPE_WATCH_REPO=/path cargo test -- --ignored --nocapture"]
async fn measure_watching_a_real_repository() {
    let path = std::env::var_os("GITPOULPE_WATCH_REPO").expect("GITPOULPE_WATCH_REPO");
    let info = session(Path::new(&path)).await;
    let started = std::time::Instant::now();
    let (watcher, _rx) = start(&info);
    println!(
        "watching {} directories, started in {:?}",
        watcher.watched_directories(),
        started.elapsed()
    );
}
