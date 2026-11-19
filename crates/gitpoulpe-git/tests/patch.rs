mod common;

use std::path::Path;

use common::{commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{ApplyMode, Cancel, Git, PatchError, PatchOutcome, check_patch_paths};

fn engine() -> Git {
    Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
}

async fn open(git: &Git, dir: &Path) -> SessionInfo {
    git.open(dir, &Cancel::new()).await.unwrap()
}

fn text(dir: &Path, args: &[&str]) -> String {
    String::from_utf8(run_git(dir, args)).unwrap()
}

fn base() -> tempfile::TempDir {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "one\ntwo\nthree\n");
    write(p, "b.txt", "alpha\nbeta\n");
    write(p, "keep.txt", "keep\n");
    commit_all(p, "init");
    repo
}

fn patch_of(source: &Path, edit: impl Fn(&Path)) -> Vec<u8> {
    edit(source);
    run_git(source, &["diff", "--binary"])
}

fn clone_of(origin: &Path) -> tempfile::TempDir {
    let clone = tempfile::tempdir().unwrap();
    run_git(
        clone.path(),
        &["clone", "-q", &format!("file://{}", origin.display()), "."],
    );
    clone
}

fn two_file_patch(origin: &Path) -> Vec<u8> {
    let source = clone_of(origin);
    patch_of(source.path(), |p| {
        write(p, "a.txt", "one\nTWO\nthree\nfour\n");
        write(p, "b.txt", "alpha\nbeta\ngamma\n");
        write(p, "new.txt", "brand new\n");
    })
}

#[tokio::test]
async fn the_preview_lists_every_targeted_file_with_its_counts_before_anything_is_applied() {
    let repo = base();
    let patch = two_file_patch(repo.path());
    let git = engine();
    let info = open(&git, repo.path()).await;
    let preview = git.preview_patch(&info, &patch).await.unwrap();
    let files: Vec<(&str, u64, u64)> = preview
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.additions, f.deletions))
        .collect();
    assert_eq!(files, [("a.txt", 2, 1), ("b.txt", 1, 0)]);
    assert!(!preview.has_message);
    assert_eq!(text(repo.path(), &["status", "--porcelain"]), "");
}

#[tokio::test]
async fn each_mode_touches_only_what_it_promises() {
    let git = engine();
    for (mode, tree_changed, index_changed) in [
        (ApplyMode::WorkingTree, true, false),
        (ApplyMode::IndexAndTree, true, true),
        (ApplyMode::IndexOnly, false, true),
    ] {
        let origin = base();
        let patch = two_file_patch(origin.path());
        let target = clone_of(origin.path());
        let info = open(&git, target.path()).await;
        git.check_patch(&info, &patch, mode).await.unwrap();
        let outcome = git.apply_patch(&info, &patch, mode, false).await.unwrap();
        assert!(matches!(&outcome, PatchOutcome::Applied(files) if files.len() == 2));
        let tree = std::fs::read_to_string(target.path().join("a.txt")).unwrap();
        assert_eq!(tree.contains("TWO"), tree_changed, "{mode:?}");
        assert_eq!(
            !text(target.path(), &["diff", "--cached", "--name-only"]).is_empty(),
            index_changed,
            "{mode:?}"
        );
        assert!(target.path().join("keep.txt").exists());
    }
}

#[tokio::test]
async fn a_patch_that_fails_halfway_changes_nothing_at_all() {
    let origin = base();
    let patch = two_file_patch(origin.path());
    let target = clone_of(origin.path());
    write(target.path(), "b.txt", "completely different\n");
    run_git(target.path(), &["commit", "-qam", "diverge"]);
    let before = text(target.path(), &["status", "--porcelain"]);
    let a_before = std::fs::read(target.path().join("a.txt")).unwrap();
    let git = engine();
    let info = open(&git, target.path()).await;
    assert!(matches!(
        git.check_patch(&info, &patch, ApplyMode::WorkingTree).await,
        Err(PatchError::Refused(_))
    ));
    let refused = git
        .apply_patch(&info, &patch, ApplyMode::WorkingTree, false)
        .await;
    assert!(matches!(refused, Err(PatchError::Refused(_))));
    assert_eq!(text(target.path(), &["status", "--porcelain"]), before);
    assert_eq!(
        std::fs::read(target.path().join("a.txt")).unwrap(),
        a_before
    );
    assert!(!target.path().join("new.txt").exists());
}

#[tokio::test]
async fn dangerous_paths_are_refused_before_git_runs_and_nothing_is_written_outside() {
    let repo = base();
    let outside = tempfile::tempdir().unwrap();
    let escape = format!(
        "../{}/evil.txt",
        outside.path().file_name().unwrap().to_str().unwrap()
    );
    let git = engine();
    let info = open(&git, repo.path()).await;
    for path in [
        escape.as_str(),
        "/etc/gitpoulpe-test",
        ".git/hooks/pre-commit",
        "sub/../../x.txt",
        "dir/.GIT/config",
        "C:/Windows/x.txt",
        "dir\\..\\x.txt",
        "nul\0byte.txt",
    ] {
        let patch = format!(
            "diff --git a/{path} b/{path}\nnew file mode 100755\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n+#!/bin/sh\n"
        );
        assert!(
            matches!(
                git.preview_patch(&info, patch.as_bytes()).await,
                Err(PatchError::Unsafe(_))
            ),
            "{path}"
        );
        assert!(matches!(
            git.apply_patch(&info, patch.as_bytes(), ApplyMode::WorkingTree, false)
                .await,
            Err(PatchError::Unsafe(_))
        ));
    }
    assert!(!outside.path().join("evil.txt").exists());
    assert!(!repo.path().join(".git/hooks/pre-commit").exists());
    assert!(check_patch_paths(b"--- a/src/main.rs\n+++ b/src/main.rs\n").is_ok());
    assert!(check_patch_paths(b"--- /dev/null\n+++ b/new.rs\n").is_ok());
    assert!(matches!(
        check_patch_paths(b"rename from ok.txt\nrename to ../bad.txt\n"),
        Err(PatchError::Unsafe(_))
    ));
}

#[tokio::test]
async fn input_that_is_empty_huge_or_not_a_patch_is_refused_with_its_own_reason() {
    let repo = base();
    let git = engine();
    let info = open(&git, repo.path()).await;
    assert_eq!(
        git.preview_patch(&info, b"  \n").await.unwrap_err(),
        PatchError::Empty
    );
    assert_eq!(
        git.preview_patch(&info, b"just some text\n")
            .await
            .unwrap_err(),
        PatchError::NotAPatch
    );
    let huge = vec![b'x'; gitpoulpe_git::PATCH_LIMIT + 1];
    assert_eq!(
        git.preview_patch(&info, &huge).await.unwrap_err(),
        PatchError::TooLarge
    );
}

#[tokio::test]
async fn a_conflicting_patch_is_refused_or_handed_to_conflict_resolution_never_half_applied() {
    let origin = base();
    let patch = {
        let source = clone_of(origin.path());
        patch_of(source.path(), |p| {
            write(p, "a.txt", "one\nTWO from patch\nthree\n")
        })
    };
    let target = clone_of(origin.path());
    write(target.path(), "a.txt", "one\nTWO local\nthree\n");
    run_git(target.path(), &["commit", "-qam", "local change"]);
    let git = engine();
    let info = open(&git, target.path()).await;
    let refused = git
        .apply_patch(&info, &patch, ApplyMode::IndexAndTree, false)
        .await;
    assert!(matches!(refused, Err(PatchError::Refused(_))));
    assert_eq!(text(target.path(), &["status", "--porcelain"]), "");
    let outcome = git
        .apply_patch(&info, &patch, ApplyMode::IndexAndTree, true)
        .await
        .unwrap();
    assert_eq!(outcome, PatchOutcome::Conflicts(vec!["a.txt".to_owned()]));
    let tree = std::fs::read_to_string(target.path().join("a.txt")).unwrap();
    assert!(
        tree.contains("<<<<<<<") && tree.contains("TWO from patch") && tree.contains("TWO local")
    );
    assert!(text(target.path(), &["status", "--porcelain"]).starts_with("UU a.txt"));
}

#[tokio::test]
async fn a_mailbox_patch_keeps_its_files_and_says_its_message_is_ignored() {
    let origin = base();
    let source = clone_of(origin.path());
    write(source.path(), "b.txt", "alpha\nbeta\nmail\n");
    run_git(source.path(), &["commit", "-qam", "From a mailbox"]);
    let mail = run_git(source.path(), &["format-patch", "-1", "--stdout"]);
    let git = engine();
    let info = open(&git, origin.path()).await;
    let preview = git.preview_patch(&info, &mail).await.unwrap();
    assert!(preview.has_message);
    assert_eq!(preview.files.len(), 1);
    git.apply_patch(&info, &mail, ApplyMode::WorkingTree, false)
        .await
        .unwrap();
    assert!(
        std::fs::read_to_string(origin.path().join("b.txt"))
            .unwrap()
            .contains("mail")
    );
}

#[tokio::test]
async fn binary_files_travel_in_the_patch() {
    let origin = base();
    let source = clone_of(origin.path());
    std::fs::write(source.path().join("blob.bin"), [0u8, 1, 2, 255, 0]).unwrap();
    run_git(source.path(), &["add", "-N", "blob.bin"]);
    let patch = run_git(source.path(), &["diff", "--binary"]);
    let git = engine();
    let info = open(&git, origin.path()).await;
    let preview = git.preview_patch(&info, &patch).await.unwrap();
    assert!(preview.files[0].binary);
    git.apply_patch(&info, &patch, ApplyMode::WorkingTree, false)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(origin.path().join("blob.bin")).unwrap(),
        [0u8, 1, 2, 255, 0]
    );
}
