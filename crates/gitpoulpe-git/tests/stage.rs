mod common;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use common::*;
use gitpoulpe_core::{
    Generation, Refusal, SessionInfo, StageOutcome, StageTarget, StatusSnapshot, WriteError,
};
use gitpoulpe_git::{Cancel, Git};

async fn open(dir: &Path) -> (SessionInfo, StatusSnapshot) {
    let git = Git::new();
    let info = git.open(dir, &Cancel::new()).await.unwrap();
    let snapshot = git
        .read_status(&info, Generation::default().next(), &Cancel::new())
        .await
        .unwrap();
    (info, snapshot)
}

fn targets(snapshot: &StatusSnapshot, paths: &[&Path]) -> Vec<StageTarget> {
    paths
        .iter()
        .map(|path| {
            let entry = snapshot
                .entries
                .iter()
                .find(|e| e.path == *path)
                .unwrap_or_else(|| panic!("{path:?} is not in the snapshot"))
                .clone();
            StageTarget {
                stamp: snapshot.stamps.get(&entry.path).copied(),
                index_id: snapshot.index_ids.get(&entry.path).cloned(),
                entry,
            }
        })
        .collect()
}

async fn stage(
    info: &SessionInfo,
    snapshot: &StatusSnapshot,
    paths: &[&Path],
) -> Result<StageOutcome, WriteError> {
    Git::new()
        .stage(info, &targets(snapshot, paths), &Cancel::new())
        .await
}

fn index(dir: &Path) -> Vec<u8> {
    git(dir, &["ls-files", "--stage", "-z"])
}

fn worktree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(&path).unwrap_or_default(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn names(bytes: Vec<u8>) -> Vec<PathBuf> {
    sorted(nul_paths(&bytes))
}

fn base_repo() -> tempfile::TempDir {
    let repo = init_repo();
    for name in ["a.txt", "b.txt", "c.txt", "gone.txt", "old.txt"] {
        write(repo.path(), name, &format!("{name}\n"));
    }
    commit_all(repo.path(), "base");
    repo
}

#[tokio::test]
async fn staging_one_file_then_a_selection_puts_exactly_that_content_in_the_index() {
    let repo = base_repo();
    let dir = repo.path();
    write(dir, "a.txt", "a changed\n");
    write(dir, "b.txt", "b changed\n");
    write(dir, "c.txt", "c changed\n");
    std::fs::remove_file(dir.join("gone.txt")).unwrap();
    write(dir, "new.txt", "new\n");
    let before_worktree = worktree(dir);
    let before_index = index(dir);

    let (info, snapshot) = open(dir).await;
    let outcome = stage(&info, &snapshot, &[Path::new("a.txt")])
        .await
        .unwrap();
    assert_eq!(outcome, StageOutcome::Applied { staged: 1 });
    assert_eq!(
        git(dir, &["show", ":a.txt"]),
        b"a changed\n",
        "the index holds the working content"
    );
    assert_ne!(index(dir), before_index);

    let (info, snapshot) = open(dir).await;
    let selection = [
        Path::new("b.txt"),
        Path::new("gone.txt"),
        Path::new("new.txt"),
    ];
    assert_eq!(
        stage(&info, &snapshot, &selection).await.unwrap(),
        StageOutcome::Applied { staged: 3 }
    );
    assert_eq!(git(dir, &["show", ":b.txt"]), b"b changed\n");
    assert_eq!(git(dir, &["show", ":new.txt"]), b"new\n");
    assert!(
        !names(git(dir, &["ls-files", "-z"])).contains(&PathBuf::from("gone.txt")),
        "the deletion is staged"
    );
    assert_eq!(
        worktree(dir),
        before_worktree,
        "the working directory is untouched"
    );
}

#[tokio::test]
async fn files_that_were_not_chosen_keep_their_index_entry_and_their_local_changes() {
    let repo = base_repo();
    let dir = repo.path();
    write(dir, "a.txt", "a changed\n");
    write(dir, "b.txt", "b changed\n");
    write(dir, "c.txt", "c changed\n");
    let before_c = git(dir, &["ls-files", "--stage", "-z", "--", "c.txt"]);

    let (info, snapshot) = open(dir).await;
    stage(&info, &snapshot, &[Path::new("a.txt")])
        .await
        .unwrap();

    assert_eq!(
        git(dir, &["ls-files", "--stage", "-z", "--", "c.txt"]),
        before_c
    );
    assert_eq!(
        names(git(dir, &["diff", "--name-only", "-z"])),
        vec![PathBuf::from("b.txt"), PathBuf::from("c.txt")]
    );
    assert_eq!(std::fs::read(dir.join("c.txt")).unwrap(), b"c changed\n");
}

#[tokio::test]
async fn a_file_changed_after_it_was_displayed_is_refused_and_the_index_is_left_alone() {
    let repo = base_repo();
    let dir = repo.path();
    write(dir, "a.txt", "a changed\n");
    write(dir, "b.txt", "b changed\n");
    let (info, snapshot) = open(dir).await;
    let before = index(dir);

    write(dir, "a.txt", "a changed again, longer\n");
    let outcome = stage(&info, &snapshot, &[Path::new("a.txt"), Path::new("b.txt")])
        .await
        .unwrap();
    assert_eq!(
        outcome,
        StageOutcome::Stale {
            paths: vec![PathBuf::from("a.txt")]
        },
        "only the changed file is reported"
    );
    assert_eq!(
        index(dir),
        before,
        "nothing is staged, not even the unchanged file"
    );
}

#[tokio::test]
async fn an_index_changed_after_it_was_displayed_is_refused() {
    let repo = base_repo();
    let dir = repo.path();
    write(dir, "a.txt", "a changed\n");
    let (info, snapshot) = open(dir).await;

    git(dir, &["add", "a.txt"]);
    let before = index(dir);
    let outcome = stage(&info, &snapshot, &[Path::new("a.txt")])
        .await
        .unwrap();
    assert_eq!(
        outcome,
        StageOutcome::Stale {
            paths: vec![PathBuf::from("a.txt")]
        }
    );
    assert_eq!(index(dir), before);
}

#[tokio::test]
async fn a_conflicted_file_is_refused_instead_of_being_marked_resolved() {
    let repo = base_repo();
    let dir = repo.path();
    git(dir, &["checkout", "-q", "-b", "side"]);
    write(dir, "a.txt", "side\n");
    commit_all(dir, "side");
    git(dir, &["checkout", "-q", "main"]);
    write(dir, "a.txt", "main\n");
    commit_all(dir, "main");
    let _ = git_output(dir, &["merge", "side"]);
    let (info, snapshot) = open(dir).await;
    let before = index(dir);

    let outcome = stage(&info, &snapshot, &[Path::new("a.txt")])
        .await
        .unwrap();
    assert_eq!(
        outcome,
        StageOutcome::Refused {
            path: PathBuf::from("a.txt"),
            reason: Refusal::Conflict
        }
    );
    assert_eq!(index(dir), before);
}

#[tokio::test]
async fn unusual_paths_are_staged_literally_and_look_alikes_are_left_alone() {
    let repo = init_repo();
    let dir = repo.path();
    let odd: Vec<OsString> = vec![
        "star*.txt".into(),
        "-dash.txt".into(),
        "with space.txt".into(),
        "keep[1].txt".into(),
        OsString::from_vec(b"bytes-\xff.txt".to_vec()),
        OsString::from_vec(b"new\nline.txt".to_vec()),
    ];
    for name in &odd {
        std::fs::write(dir.join(name), "x\n").unwrap();
    }
    std::fs::write(dir.join("starX.txt"), "look-alike\n").unwrap();
    std::fs::write(dir.join("keep1.txt"), "look-alike\n").unwrap();
    let (info, snapshot) = open(dir).await;
    let chosen: Vec<PathBuf> = odd.iter().map(PathBuf::from).collect();
    let refs: Vec<&Path> = chosen.iter().map(PathBuf::as_path).collect();

    assert_eq!(
        stage(&info, &snapshot, &refs).await.unwrap(),
        StageOutcome::Applied { staged: 6 }
    );
    let staged = names(git(dir, &["ls-files", "-z"]));
    assert_eq!(staged, sorted(chosen));
    assert!(!staged.contains(&PathBuf::from("starX.txt")));
    assert!(!staged.contains(&PathBuf::from("keep1.txt")));
}

#[tokio::test]
async fn the_content_is_the_one_git_would_stage_including_configured_clean_filters() {
    let repo = init_repo();
    let dir = repo.path();
    git(dir, &["config", "filter.up.clean", "tr a-z A-Z"]);
    write(dir, ".gitattributes", "*.up filter=up\n");
    write(dir, "f.up", "hello\n");
    let (info, snapshot) = open(dir).await;
    stage(&info, &snapshot, &[Path::new("f.up")]).await.unwrap();
    assert_eq!(git(dir, &["show", ":f.up"]), b"HELLO\n");
}

#[tokio::test]
async fn a_locked_index_is_reported_with_its_path_and_the_lock_is_never_removed() {
    let repo = base_repo();
    let dir = repo.path();
    write(dir, "a.txt", "a changed\n");
    let (info, snapshot) = open(dir).await;
    let lock = dir.join(".git/index.lock");
    std::fs::write(&lock, b"").unwrap();

    let error = stage(&info, &snapshot, &[Path::new("a.txt")])
        .await
        .unwrap_err();
    let WriteError::IndexLocked(path) = error else {
        panic!("expected IndexLocked, got {error:?}")
    };
    assert!(path.ends_with(".git/index.lock"), "{path}");
    assert!(lock.exists(), "the foreign lock is left in place");
}

#[tokio::test]
async fn a_selection_with_nothing_to_stage_or_no_targets_changes_nothing() {
    let repo = base_repo();
    let dir = repo.path();
    write(dir, "a.txt", "a changed\n");
    git(dir, &["add", "a.txt"]);
    let (info, snapshot) = open(dir).await;
    let before = index(dir);
    let outcome = stage(&info, &snapshot, &[Path::new("a.txt")])
        .await
        .unwrap();
    assert_eq!(
        outcome,
        StageOutcome::Refused {
            path: PathBuf::from("a.txt"),
            reason: Refusal::NothingToStage
        }
    );
    assert_eq!(
        Git::new().stage(&info, &[], &Cancel::new()).await.unwrap(),
        StageOutcome::Applied { staged: 0 }
    );
    assert_eq!(index(dir), before);
}
