mod common;

use std::path::Path;

use common::{commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{ApplyMode, Cancel, ExportError, ExportSource, Git, write_export};

fn engine() -> Git {
    Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
}

async fn open(git: &Git, dir: &Path) -> SessionInfo {
    git.open(dir, &Cancel::new()).await.unwrap()
}

fn base() -> tempfile::TempDir {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "one\ntwo\nthree\n");
    write(p, "keep.txt", "keep\n");
    commit_all(p, "init");
    repo
}

fn clone_of(origin: &Path) -> tempfile::TempDir {
    let clone = tempfile::tempdir().unwrap();
    run_git(
        clone.path(),
        &["clone", "-q", origin.to_str().unwrap(), "."],
    );
    clone
}

#[tokio::test]
async fn each_source_round_trips_into_a_compatible_clone() {
    let git = engine();
    let repo = base();
    let p = repo.path();
    let target = clone_of(p);
    let t = open(&git, target.path()).await;

    write(p, "a.txt", "one\nTWO\nthree\n");
    std::fs::write(p.join("bin.dat"), [0u8, 1, 2, 255]).unwrap();
    run_git(p, &["add", "bin.dat"]);
    let info = open(&git, p).await;

    let tree = git
        .export_patch(&info, &ExportSource::WorkingTree)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&tree).contains("+TWO"));
    assert!(!String::from_utf8_lossy(&tree).contains("bin.dat"));
    git.apply_patch(&t, &tree, ApplyMode::WorkingTree, false)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(target.path().join("a.txt")).unwrap(),
        "one\nTWO\nthree\n"
    );

    let staged = git.export_patch(&info, &ExportSource::Index).await.unwrap();
    assert!(String::from_utf8_lossy(&staged).contains("bin.dat"));
    assert!(!String::from_utf8_lossy(&staged).contains("+TWO"));
    git.apply_patch(&t, &staged, ApplyMode::IndexAndTree, false)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(target.path().join("bin.dat")).unwrap(),
        [0u8, 1, 2, 255]
    );

    run_git(p, &["add", "-A"]);
    commit_all(p, "second");
    write(p, "keep.txt", "keep\nmore\n");
    commit_all(p, "third");
    let one = git
        .export_patch(&info, &ExportSource::Commits("HEAD".into()))
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&one).contains("Subject: [PATCH] third"));
    assert!(!String::from_utf8_lossy(&one).contains("second"));
    let range = git
        .export_patch(&info, &ExportSource::Commits("HEAD~2..HEAD".into()))
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&range);
    assert!(text.contains("second") && text.contains("third"));
    let old = clone_of(p);
    run_git(old.path(), &["reset", "-q", "--hard", "HEAD~2"]);
    let o = open(&git, old.path()).await;
    git.apply_patch(&o, &range, ApplyMode::IndexAndTree, false)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(old.path().join("keep.txt")).unwrap(),
        "keep\nmore\n"
    );
    assert_eq!(
        std::fs::read(old.path().join("bin.dat")).unwrap(),
        [0u8, 1, 2, 255]
    );
}

#[tokio::test]
async fn an_empty_selection_and_a_hostile_revision_are_refused() {
    let git = engine();
    let repo = base();
    let info = open(&git, repo.path()).await;
    for source in [ExportSource::WorkingTree, ExportSource::Index] {
        assert_eq!(
            git.export_patch(&info, &source).await,
            Err(ExportError::Nothing)
        );
    }
    for spec in ["", "--output=/tmp/x", "a b", "-1"] {
        assert!(matches!(
            git.export_patch(&info, &ExportSource::Commits(spec.into()))
                .await,
            Err(ExportError::BadRevision(_))
        ));
    }
    assert!(matches!(
        git.export_patch(&info, &ExportSource::Commits("nonexistent".into()))
            .await,
        Err(ExportError::Git(_))
    ));
}

#[test]
fn the_destination_is_validated_and_never_left_half_written() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("out.patch");
    assert!(matches!(
        write_export(Path::new("relative.patch"), b"x", false),
        Err(ExportError::Destination(_))
    ));
    assert!(matches!(
        write_export(&dir.path().join("missing/out.patch"), b"x", false),
        Err(ExportError::Destination(_))
    ));
    assert!(matches!(
        write_export(dir.path(), b"x", true),
        Err(ExportError::Destination(_))
    ));
    write_export(&file, b"first", false).unwrap();
    assert!(write_export(&file, b"second", false).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"first");
    write_export(&file, b"second", true).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"second");
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".gitpoulpe-export-")
        })
        .collect();
    assert!(leftovers.is_empty());
}
