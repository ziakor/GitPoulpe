mod common;

use std::path::{Path, PathBuf};

use common::{commit_all, git as run_git, git_output, init_repo, write};
use gitpoulpe_core::{ConflictKind, OperationKind, Resolution, SessionInfo};
use gitpoulpe_git::{Cancel, ConflictError, ConflictSide, Git};

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
    fn read(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.path().join(name)).unwrap()
    }
    async fn reopen(&mut self) {
        self.info = self.git.open(self.path(), &Cancel::new()).await.unwrap();
    }
}

const BIN_BASE: &[u8] = &[0, 1, 2, 3, 4, 5];
const BIN_MAIN: &[u8] = &[0, 9, 9, 9];
const BIN_FEATURE: &[u8] = &[0, 7, 7, 7, 7];

fn bytes(dir: &Path, name: &str, data: &[u8]) {
    std::fs::write(dir.join(name), data).unwrap();
}

async fn merge_env() -> Env {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "line1\nline2\nline3\n");
    write(p, "del.txt", "d\n");
    write(p, "mod.txt", "m\n");
    bytes(p, "bin.bin", BIN_BASE);
    commit_all(p, "base");
    run_git(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", "line1\nfeature\nline3\n");
    write(p, "both.txt", "feature add\n");
    write(p, "del.txt", "feature changed\n");
    run_git(p, &["rm", "-q", "mod.txt"]);
    bytes(p, "bin.bin", BIN_FEATURE);
    commit_all(p, "feature work");
    run_git(p, &["checkout", "-q", "main"]);
    write(p, "a.txt", "line1\nmain\nline3\n");
    write(p, "both.txt", "main add\n");
    run_git(p, &["rm", "-q", "del.txt"]);
    write(p, "mod.txt", "main changed\n");
    bytes(p, "bin.bin", BIN_MAIN);
    commit_all(p, "main work");
    let failed = git_output(p, &["merge", "feature"]);
    assert!(!failed.status.success());
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com");
    let info = git.open(p, &Cancel::new()).await.unwrap();
    Env { repo, git, info }
}

fn names(files: &[gitpoulpe_core::ConflictFile]) -> Vec<String> {
    files.iter().map(|f| f.path.display().to_string()).collect()
}

#[tokio::test]
async fn the_sides_of_a_merge_are_named_after_the_branches() {
    let e = merge_env().await;
    let op = e.git.operation_state(&e.info).await.unwrap().unwrap();
    assert_eq!(op.kind, OperationKind::Merge);
    assert!(op.ours_label.contains("main"), "{}", op.ours_label);
    assert!(op.theirs_label.contains("feature"), "{}", op.theirs_label);
}

#[tokio::test]
async fn every_kind_of_conflict_is_identified_with_binary_files_flagged() {
    let e = merge_env().await;
    let files = e.git.list_conflicts(&e.info).await.unwrap();
    let kind = |n: &str| files.iter().find(|f| f.path == Path::new(n)).unwrap().kind;
    assert_eq!(
        names(&files),
        ["a.txt", "bin.bin", "both.txt", "del.txt", "mod.txt"]
    );
    assert_eq!(kind("a.txt"), ConflictKind::BothModified);
    assert_eq!(kind("both.txt"), ConflictKind::BothAdded);
    assert_eq!(kind("del.txt"), ConflictKind::DeletedByUs);
    assert_eq!(kind("mod.txt"), ConflictKind::DeletedByThem);
    assert!(
        files
            .iter()
            .find(|f| f.path == Path::new("bin.bin"))
            .unwrap()
            .binary
    );
    assert!(
        !files
            .iter()
            .find(|f| f.path == Path::new("a.txt"))
            .unwrap()
            .binary
    );
}

#[tokio::test]
async fn the_base_and_both_versions_are_available_with_the_merged_file() {
    let e = merge_env().await;
    let v = e
        .git
        .conflict_versions(&e.info, Path::new("a.txt"))
        .await
        .unwrap();
    assert_eq!(v.base.unwrap().text.unwrap(), "line1\nline2\nline3\n");
    assert_eq!(v.ours.unwrap().text.unwrap(), "line1\nmain\nline3\n");
    assert_eq!(v.theirs.unwrap().text.unwrap(), "line1\nfeature\nline3\n");
    assert!(v.merged.unwrap().contains("<<<<<<<"));
    let b = e
        .git
        .conflict_versions(&e.info, Path::new("bin.bin"))
        .await
        .unwrap();
    assert!(b.ours.as_ref().unwrap().text.is_none());
    assert_eq!(b.ours.unwrap().size, BIN_MAIN.len() as u64);
    let d = e
        .git
        .conflict_versions(&e.info, Path::new("del.txt"))
        .await
        .unwrap();
    assert!(d.ours.is_none() && d.theirs.is_some() && d.base.is_some());
    assert_eq!(
        e.git
            .conflict_versions(&e.info, Path::new("nope.txt"))
            .await
            .unwrap_err(),
        ConflictError::NoSuchConflict
    );
}

#[tokio::test]
async fn a_side_can_be_taken_for_text_binary_and_delete_modify_conflicts() {
    let e = merge_env().await;
    e.git
        .resolve_with_side(&e.info, Path::new("a.txt"), ConflictSide::Ours)
        .await
        .unwrap();
    assert_eq!(e.read("a.txt"), b"line1\nmain\nline3\n");
    e.git
        .resolve_with_side(&e.info, Path::new("bin.bin"), ConflictSide::Theirs)
        .await
        .unwrap();
    assert_eq!(e.read("bin.bin"), BIN_FEATURE);
    e.git
        .resolve_with_side(&e.info, Path::new("del.txt"), ConflictSide::Theirs)
        .await
        .unwrap();
    assert_eq!(e.read("del.txt"), b"feature changed\n");
    e.git
        .resolve_with_side(&e.info, Path::new("mod.txt"), ConflictSide::Theirs)
        .await
        .unwrap();
    assert!(!e.path().join("mod.txt").exists());
    e.git
        .resolve_with_side(&e.info, Path::new("both.txt"), ConflictSide::Ours)
        .await
        .unwrap();
    assert!(e.git.list_conflicts(&e.info).await.unwrap().is_empty());
    assert_eq!(
        e.git
            .resolve_with_side(&e.info, Path::new("a.txt"), ConflictSide::Ours)
            .await
            .unwrap_err(),
        ConflictError::NoSuchConflict
    );
}

#[tokio::test]
async fn an_accepted_deletion_removes_the_file() {
    let e = merge_env().await;
    e.git
        .resolve_with_side(&e.info, Path::new("del.txt"), ConflictSide::Ours)
        .await
        .unwrap();
    assert!(!e.path().join("del.txt").exists());
    assert!(!e.out(&["ls-files", "del.txt"]).contains("del.txt"));
}

#[tokio::test]
async fn an_edited_result_is_written_staged_and_checked_for_markers_and_paths() {
    let e = merge_env().await;
    let a = Path::new("a.txt");
    let marked = e.read("a.txt");
    assert_eq!(
        e.git
            .resolve_with_content(&e.info, a, &marked, false)
            .await
            .unwrap_err(),
        ConflictError::MarkersRemain
    );
    for bad in ["../outside.txt", "/etc/passwd", "a/../../x"] {
        assert_eq!(
            e.git
                .resolve_with_content(&e.info, Path::new(bad), b"x", false)
                .await
                .unwrap_err(),
            ConflictError::UnsafePath,
            "{bad}"
        );
    }
    assert_eq!(
        e.git
            .resolve_with_content(&e.info, Path::new("clean.txt"), b"x", false)
            .await
            .unwrap_err(),
        ConflictError::NoSuchConflict
    );
    e.git
        .resolve_with_content(&e.info, a, b"line1\nboth\nline3\n", false)
        .await
        .unwrap();
    assert_eq!(e.read("a.txt"), b"line1\nboth\nline3\n");
    assert!(!e.out(&["ls-files", "-u", "a.txt"]).contains("a.txt"));
    assert_eq!(e.out(&["show", ":a.txt"]), "line1\nboth\nline3");
}

#[tokio::test]
async fn markers_can_be_kept_only_when_explicitly_allowed() {
    let e = merge_env().await;
    let marked = e.read("a.txt");
    e.git
        .resolve_with_content(&e.info, Path::new("a.txt"), &marked, true)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&e.read("a.txt")).contains("<<<<<<<"));
}

#[tokio::test]
async fn a_resolved_file_edited_again_is_stale_until_validated_again() {
    let e = merge_env().await;
    let remembered: Vec<PathBuf> = e
        .git
        .list_conflicts(&e.info)
        .await
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    e.git
        .resolve_with_content(&e.info, Path::new("a.txt"), b"one\n", false)
        .await
        .unwrap();
    let state = |states: &[(PathBuf, Resolution)], n: &str| {
        states.iter().find(|(p, _)| p == Path::new(n)).unwrap().1
    };
    let states = e.git.resolution_states(&e.info, &remembered).await.unwrap();
    assert_eq!(state(&states, "a.txt"), Resolution::Resolved);
    assert_eq!(state(&states, "both.txt"), Resolution::Unresolved);
    write(e.path(), "a.txt", "edited later\n");
    let states = e.git.resolution_states(&e.info, &remembered).await.unwrap();
    assert_eq!(state(&states, "a.txt"), Resolution::Stale);
    e.git
        .revalidate_resolved(&e.info, Path::new("a.txt"))
        .await
        .unwrap();
    let states = e.git.resolution_states(&e.info, &remembered).await.unwrap();
    assert_eq!(state(&states, "a.txt"), Resolution::Resolved);
    assert_eq!(e.out(&["show", ":a.txt"]), "edited later");
}

async fn resolve_all(e: &Env) -> Vec<PathBuf> {
    let remembered: Vec<PathBuf> = e
        .git
        .list_conflicts(&e.info)
        .await
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    for f in &remembered {
        e.git
            .resolve_with_side(&e.info, &f.clone(), ConflictSide::Theirs)
            .await
            .unwrap();
    }
    remembered
}

#[tokio::test]
async fn continuing_is_refused_until_everything_is_really_resolved_then_commits_the_merge() {
    let e = merge_env().await;
    let remembered: Vec<PathBuf> = e
        .git
        .list_conflicts(&e.info)
        .await
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    let head = e.out(&["rev-parse", "HEAD"]);
    let error = e
        .git
        .continue_operation(&e.info, None, &remembered)
        .await
        .unwrap_err();
    assert!(matches!(error, ConflictError::NotReady(_)), "{error:?}");
    for f in &remembered[..4] {
        e.git
            .resolve_with_side(&e.info, f, ConflictSide::Theirs)
            .await
            .unwrap();
    }
    assert!(matches!(
        e.git.continue_operation(&e.info, None, &remembered).await,
        Err(ConflictError::NotReady(_))
    ));
    e.git
        .resolve_with_side(&e.info, &remembered[4], ConflictSide::Theirs)
        .await
        .unwrap();
    write(e.path(), "a.txt", "touched after resolution\n");
    let stale = e
        .git
        .continue_operation(&e.info, None, &remembered)
        .await
        .unwrap_err();
    assert!(
        matches!(&stale, ConflictError::NotReady(m) if m.contains("modifié après")),
        "{stale:?}"
    );
    e.git
        .revalidate_resolved(&e.info, Path::new("a.txt"))
        .await
        .unwrap();
    let done = e
        .git
        .continue_operation(&e.info, None, &remembered)
        .await
        .unwrap();
    assert!(done.finished);
    assert!(e.git.operation_state(&e.info).await.unwrap().is_none());
    let parents = e.out(&["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(parents.split(' ').count(), 3, "{parents}");
    assert!(parents.contains(&head));
    assert!(
        e.out(&["log", "-1", "--format=%s"])
            .starts_with("Merge branch 'feature'")
    );
    assert_eq!(e.read("a.txt"), b"touched after resolution\n");
}

#[tokio::test]
async fn aborting_restores_the_branch_keeps_untracked_work_and_saves_the_resolution_work() {
    let mut e = merge_env().await;
    let head = e.out(&["rev-parse", "HEAD"]);
    write(e.path(), "notes.txt", "untracked and mine\n");
    let remembered: Vec<PathBuf> = e
        .git
        .list_conflicts(&e.info)
        .await
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    e.git
        .resolve_with_content(
            &e.info,
            Path::new("a.txt"),
            b"my careful resolution\n",
            false,
        )
        .await
        .unwrap();
    let report = e.git.abort_operation(&e.info, &remembered).await.unwrap();
    assert_eq!(e.out(&["rev-parse", "HEAD"]), head);
    assert!(e.git.operation_state(&e.info).await.unwrap().is_none());
    assert_eq!(e.read("notes.txt"), b"untracked and mine\n");
    assert!(
        e.out(&["status", "--porcelain"])
            .lines()
            .all(|l| l.starts_with("??"))
    );
    let backup = report.backup.expect("work was saved");
    assert_eq!(
        std::fs::read(backup.join("a.txt")).unwrap(),
        b"my careful resolution\n"
    );
    assert!(report.saved >= 1);
    e.reopen().await;
    assert!(e.git.list_conflicts(&e.info).await.unwrap().is_empty());
}

#[tokio::test]
async fn nothing_is_offered_without_an_active_operation() {
    let e = merge_env().await;
    resolve_all(&e).await;
    e.git.continue_operation(&e.info, None, &[]).await.unwrap();
    assert_eq!(
        e.git
            .continue_operation(&e.info, None, &[])
            .await
            .unwrap_err(),
        ConflictError::NoOperation
    );
    assert_eq!(
        e.git.abort_operation(&e.info, &[]).await.unwrap_err(),
        ConflictError::NoOperation
    );
}

async fn rebase_env() -> Env {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "base\n");
    commit_all(p, "base");
    run_git(p, &["checkout", "-q", "-b", "topic"]);
    write(p, "a.txt", "topic change\n");
    commit_all(p, "topic commit");
    run_git(p, &["checkout", "-q", "main"]);
    write(p, "a.txt", "main change\n");
    commit_all(p, "main commit");
    run_git(p, &["checkout", "-q", "topic"]);
    let failed = git_output(p, &["rebase", "main"]);
    assert!(!failed.status.success());
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com");
    let info = git.open(p, &Cancel::new()).await.unwrap();
    Env { repo, git, info }
}

#[tokio::test]
async fn in_a_rebase_the_sides_are_swapped_and_named_accordingly() {
    let e = rebase_env().await;
    let op = e.git.operation_state(&e.info).await.unwrap().unwrap();
    assert_eq!(op.kind, OperationKind::Rebase);
    assert!(
        op.ours_label.contains("base du rebase"),
        "{}",
        op.ours_label
    );
    assert!(
        op.theirs_label.contains("topic commit"),
        "{}",
        op.theirs_label
    );
    let v = e
        .git
        .conflict_versions(&e.info, Path::new("a.txt"))
        .await
        .unwrap();
    assert_eq!(v.ours.unwrap().text.unwrap(), "main change\n");
    assert_eq!(v.theirs.unwrap().text.unwrap(), "topic change\n");
}

#[tokio::test]
async fn a_rebase_conflict_can_be_resolved_and_continued() {
    let e = rebase_env().await;
    let remembered = vec![PathBuf::from("a.txt")];
    e.git
        .resolve_with_content(&e.info, Path::new("a.txt"), b"combined\n", false)
        .await
        .unwrap();
    let done = e
        .git
        .continue_operation(&e.info, None, &remembered)
        .await
        .unwrap();
    assert!(done.finished, "{}", done.message);
    assert!(e.git.operation_state(&e.info).await.unwrap().is_none());
    assert_eq!(e.out(&["symbolic-ref", "--short", "HEAD"]), "topic");
    assert_eq!(e.out(&["show", "HEAD:a.txt"]), "combined");
    assert_eq!(
        e.out(&["rev-parse", "HEAD~1"]),
        e.out(&["rev-parse", "main"])
    );
}

#[tokio::test]
async fn cherry_pick_and_revert_conflicts_are_named_for_their_operation() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "one\n");
    commit_all(p, "base");
    run_git(p, &["checkout", "-q", "-b", "side"]);
    write(p, "a.txt", "side\n");
    commit_all(p, "side commit");
    run_git(p, &["checkout", "-q", "main"]);
    write(p, "a.txt", "main\n");
    commit_all(p, "main commit");
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null");
    assert!(!git_output(p, &["cherry-pick", "side"]).status.success());
    let info = git.open(p, &Cancel::new()).await.unwrap();
    let op = git.operation_state(&info).await.unwrap().unwrap();
    assert_eq!(op.kind, OperationKind::CherryPick);
    assert!(
        op.theirs_label.contains("side commit"),
        "{}",
        op.theirs_label
    );
    git.abort_operation(&info, &[]).await.unwrap();
    write(p, "a.txt", "changed again\n");
    commit_all(p, "later");
    assert!(!git_output(p, &["revert", "HEAD~1"]).status.success());
    let op = git.operation_state(&info).await.unwrap().unwrap();
    assert_eq!(op.kind, OperationKind::Revert);
    assert!(
        op.theirs_label.contains("annulation"),
        "{}",
        op.theirs_label
    );
    git.abort_operation(&info, &[]).await.unwrap();
}

#[tokio::test]
async fn a_result_is_never_written_through_a_symbolic_link() {
    let repo = init_repo();
    let p = repo.path();
    std::fs::create_dir(p.join("sub")).unwrap();
    write(p, "sub/f.txt", "one\n");
    commit_all(p, "base");
    run_git(p, &["checkout", "-q", "-b", "side"]);
    write(p, "sub/f.txt", "side\n");
    commit_all(p, "side");
    run_git(p, &["checkout", "-q", "main"]);
    write(p, "sub/f.txt", "main\n");
    commit_all(p, "main");
    assert!(!git_output(p, &["merge", "side"]).status.success());
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null");
    let info = git.open(p, &Cancel::new()).await.unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::remove_dir_all(p.join("sub")).unwrap();
    std::os::unix::fs::symlink(outside.path(), p.join("sub")).unwrap();
    let error = git
        .resolve_with_content(&info, Path::new("sub/f.txt"), b"escaped\n", false)
        .await
        .unwrap_err();
    assert_eq!(error, ConflictError::Symlink);
    assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    std::fs::remove_file(p.join("sub")).unwrap();
    std::fs::create_dir(p.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(p.join("elsewhere"), p.join("sub")).unwrap();
    let error = git
        .resolve_with_content(&info, Path::new("sub/f.txt"), b"redirected\n", false)
        .await
        .unwrap_err();
    assert_eq!(error, ConflictError::Symlink);
    assert!(
        std::fs::read_dir(p.join("elsewhere"))
            .unwrap()
            .next()
            .is_none()
    );
}
