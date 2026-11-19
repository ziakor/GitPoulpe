mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::{commit_all, git as run_git, git_output, init_repo, write};
use gitpoulpe_git::{Bounded, BoundedError, Cancel, Git, OverviewError, parse_overview};

fn engine() -> Git {
    Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
}

#[test]
fn the_porcelain_output_is_counted_per_kind_including_renames_and_conflicts() {
    let raw = b"# branch.oid abc123\0# branch.head feature\0# branch.upstream origin/feature\0# branch.ab +2 -3\0\
1 M. N... 100644 100644 100644 a b a.txt\0\
1 .M N... 100644 100644 100644 a b b.txt\0\
1 MM N... 100644 100644 100644 a b c.txt\0\
2 R. N... 100644 100644 100644 a b R100 new.txt\0? sneaky.txt\0\
u UU N... 100644 100644 100644 100644 a b c conflict.txt\0\
? loose.txt\0? other.txt\0! ignored.txt\0";
    let overview = parse_overview(raw);
    assert_eq!(overview.branch.as_deref(), Some("feature"));
    assert_eq!(overview.upstream.as_deref(), Some("origin/feature"));
    assert_eq!((overview.ahead, overview.behind), (Some(2), Some(3)));
    assert_eq!(overview.head_oid.as_deref(), Some("abc123"));
    assert_eq!((overview.staged, overview.unstaged), (3, 2));
    assert_eq!((overview.untracked, overview.conflicts), (2, 1));
    let detached = parse_overview(b"# branch.oid abc\0# branch.head (detached)\0");
    assert!(detached.branch.is_none() && detached.ahead.is_none());
    let fresh = parse_overview(b"# branch.oid (initial)\0# branch.head main\0");
    assert!(fresh.head_oid.is_none() && fresh.branch.as_deref() == Some("main"));
}

#[tokio::test]
async fn a_real_repository_is_summarised_with_branch_changes_and_distance_from_its_upstream() {
    let origin = init_repo();
    write(origin.path(), "a.txt", "one\n");
    write(origin.path(), "b.txt", "one\n");
    commit_all(origin.path(), "init");
    let clone = tempfile::tempdir().unwrap();
    run_git(
        clone.path(),
        &[
            "clone",
            "-q",
            &format!("file://{}", origin.path().display()),
            ".",
        ],
    );
    write(origin.path(), "remote.txt", "r\n");
    commit_all(origin.path(), "remote work");
    run_git(clone.path(), &["fetch", "-q"]);
    write(clone.path(), "local.txt", "l\n");
    commit_all(clone.path(), "local work");
    write(clone.path(), "a.txt", "changed\n");
    run_git(clone.path(), &["add", "a.txt"]);
    write(clone.path(), "b.txt", "changed too\n");
    write(clone.path(), "new.txt", "n\n");
    let overview = engine()
        .repo_overview(clone.path(), &Cancel::new())
        .await
        .unwrap();
    assert_eq!(overview.branch.as_deref(), Some("main"));
    assert_eq!(overview.upstream.as_deref(), Some("origin/main"));
    assert_eq!((overview.ahead, overview.behind), (Some(1), Some(1)));
    assert_eq!(
        (overview.staged, overview.unstaged, overview.untracked),
        (1, 1, 1)
    );
    assert_eq!((overview.conflicts, overview.operation), (0, None));
}

#[tokio::test]
async fn a_merge_in_progress_and_its_conflicts_are_reported() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "base\n");
    commit_all(p, "base");
    run_git(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", "feature\n");
    commit_all(p, "feature");
    run_git(p, &["checkout", "-q", "main"]);
    write(p, "a.txt", "main\n");
    commit_all(p, "main");
    assert!(!git_output(p, &["merge", "feature"]).status.success());
    let overview = engine().repo_overview(p, &Cancel::new()).await.unwrap();
    assert_eq!(overview.conflicts, 1);
    assert_eq!(overview.operation, Some("fusion en cours"));
}

#[tokio::test]
async fn a_missing_folder_and_a_plain_folder_fail_apart_and_a_fresh_repository_has_no_commit() {
    let git = engine();
    let base = tempfile::tempdir().unwrap();
    assert_eq!(
        git.repo_overview(&base.path().join("nope"), &Cancel::new())
            .await
            .unwrap_err(),
        OverviewError::Missing
    );
    assert_eq!(
        git.repo_overview(base.path(), &Cancel::new())
            .await
            .unwrap_err(),
        OverviewError::NotARepository
    );
    let fresh = init_repo();
    let overview = git
        .repo_overview(fresh.path(), &Cancel::new())
        .await
        .unwrap();
    assert!(overview.head_oid.is_none());
    assert_eq!(overview.branch.as_deref(), Some("main"));
}

#[tokio::test]
async fn reading_the_status_never_rewrites_the_index_nor_runs_the_repository_configured_monitor() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "same\n");
    commit_all(p, "init");
    let index = std::fs::read(p.join(".git/index")).unwrap();
    let file = std::fs::File::options()
        .write(true)
        .open(p.join("a.txt"))
        .unwrap();
    file.set_modified(std::time::SystemTime::now() + Duration::from_secs(3600))
        .unwrap();
    drop(file);
    let marker = p.join("monitor-ran");
    std::fs::write(
        p.join("monitor.sh"),
        format!("#!/bin/sh\ntouch '{}'\nprintf '\\0'\n", marker.display()),
    )
    .unwrap();
    run_git(
        p,
        &[
            "config",
            "core.fsmonitor",
            &format!("sh {}", p.join("monitor.sh").display()),
        ],
    );
    let overview = engine().repo_overview(p, &Cancel::new()).await.unwrap();
    assert_eq!(overview.unstaged, 0);
    assert_eq!(std::fs::read(p.join(".git/index")).unwrap(), index);
    assert!(!marker.exists());
}

#[tokio::test]
async fn a_large_working_tree_is_summarised_in_bounded_time() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "seed.txt", "x\n");
    commit_all(p, "init");
    for i in 0..4000 {
        std::fs::write(p.join(format!("untracked-{i}.txt")), "u\n").unwrap();
    }
    let started = Instant::now();
    let overview = engine().repo_overview(p, &Cancel::new()).await.unwrap();
    assert_eq!(overview.untracked, 4000);
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[tokio::test]
async fn at_most_the_allowed_number_of_reads_run_together_and_all_of_them_finish() {
    let bounded = Bounded::new(3, Duration::from_secs(5));
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for i in 0..12 {
        let (bounded, running, peak) = (bounded.clone(), running.clone(), peak.clone());
        handles.push(tokio::spawn(async move {
            bounded
                .run(&Cancel::new(), |_| async move {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    running.fetch_sub(1, Ordering::SeqCst);
                    i
                })
                .await
        }));
    }
    let mut done = 0;
    for handle in handles {
        done += usize::from(handle.await.unwrap().is_ok());
    }
    assert_eq!(done, 12);
    assert!(peak.load(Ordering::SeqCst) <= 3 && peak.load(Ordering::SeqCst) >= 2);
}

#[tokio::test]
async fn a_slow_read_times_out_alone_and_never_holds_back_the_fast_ones() {
    let bounded = Bounded::new(2, Duration::from_millis(150));
    let slow = {
        let bounded = bounded.clone();
        tokio::spawn(async move {
            bounded
                .run(&Cancel::new(), |_| async {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                })
                .await
        })
    };
    let started = Instant::now();
    for _ in 0..6 {
        let value = bounded.run(&Cancel::new(), |_| async { 7 }).await.unwrap();
        assert_eq!(value, 7);
    }
    assert!(
        started.elapsed() < Duration::from_millis(140),
        "fast reads were not held back"
    );
    assert_eq!(slow.await.unwrap(), Err(BoundedError::TimedOut));
    let own = Cancel::new();
    let tight = Bounded::new(1, Duration::from_millis(30));
    let outcome = tight
        .run(&own, |_| async {
            tokio::time::sleep(Duration::from_secs(5)).await
        })
        .await;
    assert_eq!(outcome, Err(BoundedError::TimedOut));
    assert!(
        own.is_cancelled(),
        "a timed-out read is cancelled, not just forgotten"
    );
    assert!(bounded.run(&Cancel::new(), |_| async { 1 }).await.is_ok());
}

#[tokio::test]
async fn cancelling_stops_a_waiting_or_running_read_and_frees_its_slot() {
    let bounded = Bounded::new(1, Duration::from_secs(30));
    let first_cancel = Cancel::new();
    let first = {
        let (bounded, cancel) = (bounded.clone(), first_cancel.clone());
        tokio::spawn(async move {
            bounded
                .run(&cancel, |_| async {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                })
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    let waiting_cancel = Cancel::new();
    let waiting = {
        let (bounded, cancel) = (bounded.clone(), waiting_cancel.clone());
        tokio::spawn(async move { bounded.run(&cancel, |_| async { 1 }).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    waiting_cancel.cancel();
    assert_eq!(waiting.await.unwrap(), Err(BoundedError::Cancelled));
    first_cancel.cancel();
    assert_eq!(first.await.unwrap(), Err(BoundedError::Cancelled));
    assert!(bounded.run(&Cancel::new(), |_| async { 2 }).await.is_ok());
}

#[test]
fn the_remote_addresses_of_a_repository_are_listed_without_any_network_access() {
    let repo = init_repo();
    let p = repo.path();
    assert!(gitpoulpe_git::remote_urls(p).is_empty());
    run_git(
        p,
        &["remote", "add", "origin", "git@github.com:Acme/Tool.git"],
    );
    run_git(
        p,
        &[
            "remote",
            "add",
            "mirror",
            "https://gitlab.example.org/acme/platform/tool.git",
        ],
    );
    let mut urls = gitpoulpe_git::remote_urls(p);
    urls.sort();
    assert_eq!(
        urls,
        [
            "git@github.com:Acme/Tool.git",
            "https://gitlab.example.org/acme/platform/tool.git"
        ]
    );
    assert!(gitpoulpe_git::remote_urls(&p.join("nope")).is_empty());
}

#[test]
fn local_branches_and_commit_existence_are_read_without_the_network() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "x\n");
    commit_all(p, "init");
    run_git(p, &["branch", "feature/12-login"]);
    run_git(p, &["branch", "other"]);
    let mut names = gitpoulpe_git::local_branches(p);
    names.sort();
    assert_eq!(names, ["feature/12-login", "main", "other"]);
    let head = String::from_utf8(run_git(p, &["rev-parse", "HEAD"])).unwrap();
    let head = head.trim();
    assert!(gitpoulpe_git::commit_exists(p, head));
    assert!(gitpoulpe_git::commit_exists(p, &head[..8]));
    assert!(!gitpoulpe_git::commit_exists(p, &"d".repeat(40)));
    assert!(!gitpoulpe_git::commit_exists(p, "--upload-pack=x"));
    assert!(!gitpoulpe_git::commit_exists(p, "abc"));
    assert!(gitpoulpe_git::local_branches(&p.join("nope")).is_empty());
}
