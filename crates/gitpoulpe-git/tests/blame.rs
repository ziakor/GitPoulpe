mod common;

use std::path::Path;
use std::process::Command;

use common::{git as run_git, init_repo, write};
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{Cancel, Git, InspectError};

fn engine() -> Git {
    Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
}

async fn open(git: &Git, dir: &Path) -> SessionInfo {
    git.open(dir, &Cancel::new()).await.unwrap()
}

fn commit_as(dir: &Path, author: &str, message: &str) -> String {
    run_git(dir, &["add", "-A"]);
    let output = Command::new("git")
        .current_dir(dir)
        .args(["commit", "-q", "-m", message])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", author)
        .env("GIT_AUTHOR_EMAIL", "a@example.com")
        .env("GIT_COMMITTER_NAME", author)
        .env("GIT_COMMITTER_EMAIL", "a@example.com")
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&run_git(dir, &["rev-parse", "HEAD"]))
        .trim()
        .to_owned()
}

#[tokio::test]
async fn every_line_is_attributed_to_the_commit_that_wrote_it_at_the_revision_read() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "f.txt", "l1\nl2\nl3\n");
    let first = commit_as(p, "Alice", "first");
    write(p, "f.txt", "l1\nl2 changed\nl3\nl4\n");
    let second = commit_as(p, "Bob", "second subject");
    let git = engine();
    let info = open(&git, p).await;
    let head = git.blame(&info, Some(&second), "f.txt").await.unwrap();
    let seen: Vec<(u64, &str, &str, &str)> = head
        .lines
        .iter()
        .map(|l| {
            (
                l.line,
                l.oid.as_deref().unwrap(),
                l.author.as_str(),
                l.text.as_str(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            (1, first.as_str(), "Alice", "l1"),
            (2, second.as_str(), "Bob", "l2 changed"),
            (3, first.as_str(), "Alice", "l3"),
            (4, second.as_str(), "Bob", "l4"),
        ]
    );
    assert_eq!(head.lines[1].summary, "second subject");
    assert!(
        head.lines
            .iter()
            .all(|l| l.timestamp > 0 && l.path == "f.txt")
    );
    let old = git.blame(&info, Some(&first), "f.txt").await.unwrap();
    assert_eq!(old.lines.len(), 3);
    assert!(
        old.lines
            .iter()
            .all(|l| l.oid.as_deref() == Some(first.as_str()))
    );
    assert_eq!(old.lines[1].text, "l2");
}

#[tokio::test]
async fn lines_not_yet_committed_are_told_apart_from_historical_ones() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "f.txt", "l1\nl2\n");
    let first = commit_as(p, "Alice", "first");
    write(p, "f.txt", "l1\nlocal\nl2\n");
    let git = engine();
    let info = open(&git, p).await;
    let blame = git.blame(&info, None, "f.txt").await.unwrap();
    let oids: Vec<Option<&str>> = blame.lines.iter().map(|l| l.oid.as_deref()).collect();
    assert_eq!(oids, [Some(first.as_str()), None, Some(first.as_str())]);
    assert_eq!(blame.lines[1].text, "local");
    run_git(p, &["add", "f.txt"]);
    let staged = git.blame(&info, None, "f.txt").await.unwrap();
    assert!(staged.lines[1].oid.is_none());
}

#[tokio::test]
async fn a_renamed_file_keeps_the_attribution_of_its_older_lines_with_their_old_path() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "old.txt", "a\nb\nc\nd\ne\nf\n");
    let first = commit_as(p, "Alice", "first");
    run_git(p, &["mv", "old.txt", "new.txt"]);
    write(p, "new.txt", "a\nb\nc\nd\ne\nf\ng\n");
    let second = commit_as(p, "Bob", "rename and add");
    let git = engine();
    let info = open(&git, p).await;
    let blame = git.blame(&info, Some(&second), "new.txt").await.unwrap();
    assert_eq!(blame.lines[0].oid.as_deref(), Some(first.as_str()));
    assert_eq!(blame.lines[0].path, "old.txt");
    assert_eq!(blame.lines[6].oid.as_deref(), Some(second.as_str()));
    assert_eq!(blame.lines[6].path, "new.txt");
}

#[tokio::test]
async fn a_file_of_thousands_of_lines_is_attributed_completely_and_a_huge_one_is_refused() {
    let repo = init_repo();
    let p = repo.path();
    let body: String = (0..5000).map(|n| format!("line {n}\n")).collect();
    write(p, "big.txt", &body);
    let first = commit_as(p, "Alice", "big");
    let edited = body.replacen("line 2500\n", "line 2500 edited\n", 1);
    write(p, "big.txt", &edited);
    let second = commit_as(p, "Bob", "edit middle");
    std::fs::write(p.join("huge.txt"), "x".repeat(2 * 1024 * 1024 + 10)).unwrap();
    let third = commit_as(p, "Alice", "huge");
    let git = engine();
    let info = open(&git, p).await;
    let blame = git.blame(&info, Some(&second), "big.txt").await.unwrap();
    assert_eq!(blame.lines.len(), 5000);
    assert_eq!(blame.lines[2500].oid.as_deref(), Some(second.as_str()));
    assert_eq!(blame.lines[2499].oid.as_deref(), Some(first.as_str()));
    assert_eq!(blame.lines[4999].line, 5000);
    assert_eq!(
        git.blame(&info, Some(&third), "huge.txt")
            .await
            .unwrap_err(),
        InspectError::TooLarge
    );
}

#[tokio::test]
async fn unsafe_or_unknown_inputs_are_refused() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "f.txt", "x\n");
    let first = commit_as(p, "Alice", "first");
    write(p, "untracked.txt", "u\n");
    let git = engine();
    let info = open(&git, p).await;
    assert_eq!(
        git.blame(&info, Some("zz"), "f.txt").await.unwrap_err(),
        InspectError::InvalidId
    );
    assert_eq!(
        git.blame(&info, None, "/etc/passwd").await.unwrap_err(),
        InspectError::InvalidId
    );
    assert_eq!(
        git.blame(&info, Some(&first), "nope.txt")
            .await
            .unwrap_err(),
        InspectError::NotACommit
    );
    assert!(matches!(
        git.blame(&info, None, "untracked.txt").await,
        Err(InspectError::GitFailed(_))
    ));
}
