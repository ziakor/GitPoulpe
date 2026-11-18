mod common;

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{commit_all, git as run_git, git_output, init_repo, write};
use gitpoulpe_core::{Commit, GraphLayout, SessionInfo};
use gitpoulpe_git::{Cancel, Git, HistoryError};

async fn open(git: &Git, dir: &Path) -> SessionInfo {
    git.open(dir, &Cancel::new()).await.unwrap()
}

async fn read_all(git: &Git, info: &SessionInfo, page: usize) -> Result<Vec<Commit>, HistoryError> {
    let mut stream = git.history(info).await?;
    let mut all = Vec::new();
    loop {
        let chunk = stream.next_page(page).await?;
        let done = chunk.len() < page;
        all.extend(chunk);
        if done || stream.is_exhausted() {
            return Ok(all);
        }
    }
}

fn oracle(dir: &Path) -> Vec<(String, Vec<String>)> {
    let out = run_git(
        dir,
        &[
            "log",
            "--topo-order",
            "--parents",
            "--format=%H %P",
            "--branches",
            "--tags",
            "--remotes",
            "HEAD",
        ],
    );
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| {
            let mut ids = line.split_whitespace().map(str::to_owned);
            (ids.next().unwrap(), ids.collect())
        })
        .collect()
}

fn commit(dir: &Path, message: &str) {
    write(dir, &format!("f{}.txt", message.replace(' ', "_")), message);
    commit_all(dir, message);
}

fn branching_repo() -> tempfile::TempDir {
    let repo = init_repo();
    let dir = repo.path();
    commit(dir, "root");
    for round in 0..6 {
        run_git(dir, &["checkout", "-q", "-b", &format!("feature-{round}")]);
        commit(dir, &format!("feature {round} one"));
        commit(dir, &format!("feature {round} two"));
        run_git(dir, &["checkout", "-q", "main"]);
        commit(dir, &format!("main {round}"));
        run_git(
            dir,
            &[
                "merge",
                "-q",
                "--no-ff",
                "-m",
                &format!("merge feature {round}"),
                &format!("feature-{round}"),
            ],
        );
    }
    run_git(dir, &["checkout", "-q", "-b", "side-a", "HEAD~3"]);
    commit(dir, "side a");
    run_git(dir, &["checkout", "-q", "-b", "side-b", "HEAD~1"]);
    commit(dir, "side b");
    run_git(dir, &["checkout", "-q", "main"]);
    let octopus = git_output(
        dir,
        &[
            "merge", "-q", "--no-ff", "-m", "octopus", "side-a", "side-b",
        ],
    );
    assert!(
        octopus.status.success(),
        "{}",
        String::from_utf8_lossy(&octopus.stderr)
    );
    run_git(dir, &["tag", "v1", "HEAD~2"]);
    repo
}

#[tokio::test]
async fn the_order_and_the_parent_links_are_exactly_gits_including_merges() {
    let repo = branching_repo();
    let git = Git::new();
    let info = open(&git, repo.path()).await;
    let expected = oracle(repo.path());
    assert!(expected.len() > 25, "{} commits", expected.len());
    assert!(
        expected.iter().any(|(_, p)| p.len() == 2),
        "the fixture has merges"
    );
    assert!(
        expected.iter().any(|(_, p)| p.len() == 3),
        "the fixture has an octopus merge"
    );

    for page in [1, 7, 1000] {
        let got = read_all(&git, &info, page).await.unwrap();
        let got: Vec<(String, Vec<String>)> = got.into_iter().map(|c| (c.oid, c.parents)).collect();
        assert_eq!(got, expected, "page size {page}");
    }
    let all = read_all(&git, &info, 50).await.unwrap();
    let tip = all
        .iter()
        .find(|c| c.refs.iter().any(|r| r == "v1" || r == "tag: v1"))
        .is_some();
    assert!(
        tip,
        "decorations are carried: {:?}",
        all.iter().flat_map(|c| &c.refs).collect::<Vec<_>>()
    );
    assert!(all[0].subject == "octopus" && all[0].author == "t");
}

fn verify_graph(commits: &[Commit]) {
    let mut layout = GraphLayout::new();
    let mut lanes: Vec<Option<String>> = Vec::new();
    for commit in commits {
        let row = layout.push(commit);
        while lanes.len() < row.width {
            lanes.push(None);
        }
        let expected_here = lanes.get(row.column).cloned().flatten();
        assert!(
            expected_here.is_none() || expected_here.as_deref() == Some(commit.oid.as_str()),
            "{}: the lane {} carries another commit",
            commit.oid,
            row.column
        );
        for lane in &row.closing {
            assert_eq!(
                lanes[*lane].as_deref(),
                Some(commit.oid.as_str()),
                "a closing lane carries this commit"
            );
            lanes[*lane] = None;
        }
        for lane in &row.through {
            assert!(lanes[*lane].is_some(), "a through lane is occupied");
        }
        assert_eq!(row.continues, !commit.parents.is_empty());
        lanes[row.column] = commit.parents.first().cloned();
        let mut opening = row.opening.iter();
        let mut linking = row.linking.iter();
        for parent in commit.parents.iter().skip(1) {
            if let Some(existing) = lanes
                .iter()
                .position(|l| l.as_deref() == Some(parent.as_str()))
            {
                assert!(
                    linking.next().is_some_and(|l| *l == existing),
                    "{parent} is linked to its waiting lane"
                );
            } else {
                let lane = *opening.next().expect("a lane is opened for this parent");
                assert!(lanes[lane].is_none(), "the opened lane was free");
                lanes[lane] = Some(parent.clone());
            }
        }
        assert!(
            opening.next().is_none() && linking.next().is_none(),
            "no stray connector"
        );
    }
    assert!(
        lanes.iter().all(Option::is_none),
        "every parent was reached: {lanes:?}"
    );
}

#[tokio::test]
async fn the_graph_layout_reproduces_the_parent_links_of_a_real_branching_history() {
    let repo = branching_repo();
    let git = Git::new();
    let info = open(&git, repo.path()).await;
    let all = read_all(&git, &info, 9).await.unwrap();
    verify_graph(&all);
    let mut layout = GraphLayout::new();
    let widest = all.iter().map(|c| layout.push(c).width).max().unwrap();
    assert!(widest >= 3, "the history really branches ({widest} lanes)");
}

fn big_history(dir: &Path, commits: usize) {
    let mut stream = String::new();
    for n in 1..=commits {
        stream.push_str(&format!(
            "commit refs/heads/main\nmark :{n}\ncommitter t <t@example.com> {} +0000\ndata 6\nc{n:05}\n",
            1_700_000_000 + n
        ));
        if n > 1 {
            stream.push_str(&format!("from :{}\n", n - 1));
        }
        stream.push_str(&format!("M 100644 inline f.txt\ndata 2\n{}\n\n", n % 10));
    }
    let mut child = Command::new("git")
        .current_dir(dir)
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stream.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}

#[tokio::test]
async fn a_very_long_history_is_read_page_by_page_without_reading_it_all() {
    let repo = init_repo();
    let dir = repo.path();
    big_history(dir, 30_000);
    let git = Git::new();
    let info = open(&git, dir).await;

    let started = Instant::now();
    let mut stream = git.history(&info).await.unwrap();
    let first = stream.next_page(200).await.unwrap();
    assert_eq!(first.len(), 200);
    assert!(
        !stream.is_exhausted(),
        "the stream still has the rest of the history to give"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the first page is quick: {:?}",
        started.elapsed()
    );
    assert_eq!(first[0].subject, "c30000");

    let mut seen = first.len();
    loop {
        let page = stream.next_page(5_000).await.unwrap();
        seen += page.len();
        if page.len() < 5_000 {
            break;
        }
    }
    assert_eq!(seen, 30_000);
    assert!(stream.is_exhausted());
}

#[tokio::test]
async fn a_repository_without_a_commit_and_a_git_failure_are_different_states() {
    let repo = init_repo();
    let git = Git::new();
    let info = open(&git, repo.path()).await;
    assert_eq!(
        read_all(&git, &info, 10).await.unwrap_err(),
        HistoryError::NoCommits
    );

    let tools = tempfile::tempdir().unwrap();
    let script = tools.path().join("broken-git");
    std::fs::write(
        &script,
        "#!/bin/sh\ncase \"$*\" in *rev-parse*) exit 1;; esac\necho 'fatal: unable to read the object database' >&2\nexit 128\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let broken = Git::with_program(script);
    let repo = init_repo();
    commit(repo.path(), "something");
    let info = open(&git, repo.path()).await;
    let error = read_all(&broken, &info, 10).await.unwrap_err();
    let HistoryError::GitFailed(diagnostic) = error else {
        panic!("a Git failure must not look like an empty repository: {error:?}")
    };
    assert_eq!(diagnostic.exit_code, Some(128));
    assert!(
        diagnostic
            .message
            .contains("unable to read the object database"),
        "{diagnostic:?}"
    );

    let missing = Git::with_program(PathBuf::from("/nonexistent/git-program"));
    let error = missing
        .history(&info)
        .await
        .err()
        .expect("a missing program fails");
    assert!(
        matches!(error, HistoryError::GitUnavailable(_)),
        "{error:?}"
    );
    let _ = HashMap::<u8, u8>::new();
}

#[tokio::test]
async fn dropping_the_stream_stops_the_git_process() {
    let repo = init_repo();
    big_history(repo.path(), 20_000);
    let git = Git::new();
    let info = open(&git, repo.path()).await;
    let marker = "--format=%H%x1f%P%x1f%an%x1f%at%x1f%s%x1f%D".to_owned();
    let count = |dir: &Path| -> usize {
        let out = Command::new("pgrep")
            .args(["-fc", "--", &marker])
            .output()
            .unwrap();
        let _ = dir;
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(0)
    };
    let before = count(repo.path());
    let mut stream = git.history(&info).await.unwrap();
    let _ = stream.next_page(10).await.unwrap();
    assert!(count(repo.path()) > before, "the log process is running");
    drop(stream);
    for _ in 0..100 {
        if count(repo.path()) <= before {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the log process was left running");
}
