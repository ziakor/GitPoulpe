mod common;

use std::path::Path;

use common::{commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{Cancel, FileChange, Git, InspectError};

fn engine() -> Git {
    Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
}

async fn open(git: &Git, dir: &Path) -> SessionInfo {
    git.open(dir, &Cancel::new()).await.unwrap()
}

fn settle(dir: &Path) {
    run_git(dir, &["add", "-A"]);
}

fn lineage() -> tempfile::TempDir {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "one\ntwo\nthree\nfour\nfive\n");
    commit_all(p, "add a");
    run_git(p, &["mv", "a.txt", "b.txt"]);
    run_git(p, &["commit", "-qm", "rename to b"]);
    write(p, "b.txt", "one\ntwo\nthree\nfour\nfive\nsix\n");
    commit_all(p, "edit b");
    run_git(p, &["mv", "b.txt", "dir_c.txt"]);
    run_git(p, &["commit", "-qm", "rename to c"]);
    run_git(p, &["rm", "-q", "dir_c.txt"]);
    run_git(p, &["commit", "-qm", "delete c"]);
    write(p, "dir_c.txt", "brand new\n");
    commit_all(p, "recreate c");
    repo
}

#[tokio::test]
async fn a_renamed_deleted_and_recreated_file_keeps_every_step_and_each_path() {
    let repo = lineage();
    let git = engine();
    let info = open(&git, repo.path()).await;
    let history = git.file_history(&info, "dir_c.txt", 100).await.unwrap();
    let steps: Vec<(FileChange, &str, Option<&str>, &str)> = history
        .revisions
        .iter()
        .map(|r| {
            (
                r.change,
                r.path.as_str(),
                r.old_path.as_deref(),
                r.subject.as_str(),
            )
        })
        .collect();
    assert_eq!(
        steps,
        [
            (FileChange::Added, "dir_c.txt", None, "recreate c"),
            (FileChange::Deleted, "dir_c.txt", None, "delete c"),
            (
                FileChange::Renamed,
                "dir_c.txt",
                Some("b.txt"),
                "rename to c"
            ),
            (FileChange::Modified, "b.txt", None, "edit b"),
            (FileChange::Renamed, "b.txt", Some("a.txt"), "rename to b"),
            (FileChange::Added, "a.txt", None, "add a"),
        ]
    );
    assert!(!history.incomplete && !history.truncated);
    assert!(
        history
            .revisions
            .iter()
            .all(|r| r.author == "t" && r.timestamp > 0)
    );
}

#[tokio::test]
async fn a_selected_revision_opens_its_exact_content_and_diff() {
    let repo = lineage();
    let git = engine();
    let info = open(&git, repo.path()).await;
    let history = git.file_history(&info, "dir_c.txt", 100).await.unwrap();
    let by = |subject: &str| {
        history
            .revisions
            .iter()
            .find(|r| r.subject == subject)
            .unwrap()
    };
    let edit = by("edit b");
    let view = git
        .file_revision(&info, &edit.oid, &edit.path, edit.old_path.as_deref())
        .await
        .unwrap();
    assert!(view.content.as_deref().unwrap().ends_with("five\nsix\n"));
    assert!(view.diff.contains("@@") && view.diff.contains("+six") && !view.diff.contains("+five"));
    assert!(!view.deleted && !view.binary);
    let renamed = by("rename to c");
    let view = git
        .file_revision(
            &info,
            &renamed.oid,
            &renamed.path,
            renamed.old_path.as_deref(),
        )
        .await
        .unwrap();
    assert!(view.diff.contains("rename from b.txt") && view.diff.contains("rename to dir_c.txt"));
    let deleted = by("delete c");
    let view = git
        .file_revision(&info, &deleted.oid, &deleted.path, None)
        .await
        .unwrap();
    assert!(view.deleted && view.content.is_none());
    assert!(view.diff.contains("deleted file mode"));
    let recreated = by("recreate c");
    let view = git
        .file_revision(&info, &recreated.oid, &recreated.path, None)
        .await
        .unwrap();
    assert_eq!(view.content.as_deref(), Some("brand new\n"));
    assert!(view.diff.contains("new file mode"));
    let root = by("add a");
    let view = git
        .file_revision(&info, &root.oid, "a.txt", None)
        .await
        .unwrap();
    assert!(view.diff.contains("+one"));
}

#[tokio::test]
async fn the_history_is_cut_at_the_limit_and_says_so() {
    let repo = lineage();
    let git = engine();
    let info = open(&git, repo.path()).await;
    let history = git.file_history(&info, "dir_c.txt", 2).await.unwrap();
    assert_eq!(history.revisions.len(), 2);
    assert!(history.truncated && !history.incomplete);
}

#[tokio::test]
async fn a_shallow_clone_reports_an_incomplete_history_instead_of_a_false_origin() {
    let origin = init_repo();
    let p = origin.path();
    for n in 0..4 {
        write(p, "f.txt", &format!("version {n}\n"));
        commit_all(p, &format!("change {n}"));
    }
    let clone = tempfile::tempdir().unwrap();
    run_git(
        clone.path(),
        &[
            "clone",
            "-q",
            "--depth",
            "2",
            &format!("file://{}", p.display()),
            ".",
        ],
    );
    let git = engine();
    let info = open(&git, clone.path()).await;
    let history = git.file_history(&info, "f.txt", 100).await.unwrap();
    assert_eq!(history.revisions.len(), 2);
    assert!(history.incomplete);
    let full = tempfile::tempdir().unwrap();
    run_git(
        full.path(),
        &["clone", "-q", &format!("file://{}", p.display()), "."],
    );
    let info = open(&git, full.path()).await;
    let history = git.file_history(&info, "f.txt", 100).await.unwrap();
    assert_eq!(history.revisions.len(), 4);
    assert!(!history.incomplete);
}

#[tokio::test]
async fn a_path_with_no_history_is_empty_and_unsafe_inputs_are_refused() {
    let repo = lineage();
    let git = engine();
    let info = open(&git, repo.path()).await;
    let none = git
        .file_history(&info, "never/existed.txt", 10)
        .await
        .unwrap();
    assert!(none.revisions.is_empty() && !none.incomplete);
    for bad in ["", "/etc/passwd", "a\0b"] {
        assert_eq!(
            git.file_history(&info, bad, 10).await.unwrap_err(),
            InspectError::InvalidId
        );
    }
    assert_eq!(
        git.file_history(&info, "a.txt", 0).await.unwrap_err(),
        InspectError::InvalidId
    );
    let history = git.file_history(&info, "dir_c.txt", 1).await.unwrap();
    let oid = history.revisions[0].oid.clone();
    assert_eq!(
        git.file_revision(&info, "zz", "dir_c.txt", None)
            .await
            .unwrap_err(),
        InspectError::InvalidId
    );
    assert_eq!(
        git.file_revision(&info, &oid, "../x", Some(""))
            .await
            .unwrap_err(),
        InspectError::InvalidId
    );
}

#[tokio::test]
async fn binary_and_oversized_content_are_not_shown_as_text() {
    let repo = init_repo();
    let p = repo.path();
    std::fs::write(p.join("blob.bin"), [0u8, 1, 2, 3, 0, 255]).unwrap();
    std::fs::write(p.join("big.txt"), "x".repeat(1024 * 1024 + 10)).unwrap();
    settle(p);
    run_git(p, &["commit", "-qm", "add data"]);
    let git = engine();
    let info = open(&git, p).await;
    let bin = git.file_history(&info, "blob.bin", 5).await.unwrap();
    let view = git
        .file_revision(&info, &bin.revisions[0].oid, "blob.bin", None)
        .await
        .unwrap();
    assert!(view.binary && view.content.is_none());
    let big = git.file_history(&info, "big.txt", 5).await.unwrap();
    let view = git
        .file_revision(&info, &big.revisions[0].oid, "big.txt", None)
        .await
        .unwrap();
    assert!(view.content_too_large && view.content.is_none() && !view.binary);
    assert!(view.diff_truncated);
}
