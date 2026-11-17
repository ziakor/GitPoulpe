mod common;

use std::path::{Path, PathBuf};

use common::*;
use gitpoulpe_core::{
    Generation, HunkSpec, Refusal, SessionInfo, StageOutcome, StageTarget, StatusSnapshot,
    WriteError,
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

fn target(snapshot: &StatusSnapshot, path: &str) -> StageTarget {
    let entry = snapshot
        .entries
        .iter()
        .find(|e| e.path == Path::new(path))
        .unwrap_or_else(|| panic!("{path} is not in the snapshot"))
        .clone();
    StageTarget {
        stamp: snapshot.stamps.get(&entry.path).copied(),
        entry,
    }
}

fn hunks(dir: &Path, path: &str) -> Vec<HunkSpec> {
    let diff = git(dir, &["diff", "--unified=3", "--", path]);
    let mut specs = Vec::new();
    for line in diff.split(|b| *b == b'\n') {
        if let Some(rest) = line.strip_prefix(b"@@ -") {
            let text = String::from_utf8_lossy(rest).into_owned();
            let mut parts = text.split(' ');
            let range = |part: &str| {
                let part = part.trim_start_matches('+');
                match part.split_once(',') {
                    Some((a, b)) => (a.parse().unwrap(), b.parse().unwrap()),
                    None => (part.parse().unwrap(), 1u32),
                }
            };
            let (old_start, old_len) = range(parts.next().unwrap());
            let (new_start, new_len) = range(parts.next().unwrap());
            specs.push(HunkSpec {
                index: specs.len(),
                old_start,
                old_len,
                new_start,
                new_len,
            });
        }
    }
    specs
}

async fn stage_hunk(
    dir: &Path,
    path: &str,
    which: usize,
) -> (Result<StageOutcome, WriteError>, SessionInfo) {
    let (info, snapshot) = open(dir).await;
    let spec = hunks(dir, path)[which];
    let result = Git::new()
        .stage_hunk(&info, &target(&snapshot, path), &spec, &Cancel::new())
        .await;
    (result, info)
}

fn numbered(count: usize) -> Vec<String> {
    (1..=count).map(|n| format!("line {n}")).collect()
}

fn text(lines: &[String]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn three_block_repo() -> (tempfile::TempDir, Vec<String>) {
    let repo = init_repo();
    let base = numbered(40);
    write(repo.path(), "f.txt", &text(&base));
    write(repo.path(), "other.txt", "other\n");
    commit_all(repo.path(), "base");
    let mut edited = base.clone();
    edited[2] = "line 3 CHANGED".into();
    edited[19] = "line 20 CHANGED".into();
    edited[37] = "line 38 CHANGED".into();
    write(repo.path(), "f.txt", &text(&edited));
    (repo, base)
}

#[tokio::test]
async fn staging_one_block_of_three_puts_only_that_block_in_the_index() {
    let (repo, base) = three_block_repo();
    let dir = repo.path();
    assert_eq!(hunks(dir, "f.txt").len(), 3, "the fixture has three blocks");
    let worktree_before = std::fs::read(dir.join("f.txt")).unwrap();

    let (result, _) = stage_hunk(dir, "f.txt", 1).await;
    assert_eq!(result.unwrap(), StageOutcome::HunkApplied);

    let mut expected = base.clone();
    expected[19] = "line 20 CHANGED".into();
    assert_eq!(
        String::from_utf8(git(dir, &["show", ":f.txt"])).unwrap(),
        text(&expected),
        "the index holds the base plus only the middle block"
    );
    assert_eq!(std::fs::read(dir.join("f.txt")).unwrap(), worktree_before);
    let remaining = git(dir, &["diff", "--unified=0", "--", "f.txt"]);
    let remaining = String::from_utf8_lossy(&remaining);
    assert!(remaining.contains("line 3 CHANGED") && remaining.contains("line 38 CHANGED"));
    assert!(!remaining.contains("line 20 CHANGED"), "{remaining}");
}

#[tokio::test]
async fn blocks_can_be_staged_one_after_the_other_from_fresh_snapshots() {
    let (repo, _) = three_block_repo();
    let dir = repo.path();
    for _ in 0..3 {
        let (result, _) = stage_hunk(dir, "f.txt", 0).await;
        assert_eq!(result.unwrap(), StageOutcome::HunkApplied);
    }
    assert!(
        git(dir, &["diff", "--", "f.txt"]).is_empty(),
        "nothing left unstaged"
    );
    assert_eq!(
        git(dir, &["show", ":f.txt"]),
        std::fs::read(dir.join("f.txt")).unwrap()
    );
}

#[tokio::test]
async fn blocks_with_a_missing_final_newline_and_crlf_line_endings_stage_exactly() {
    let repo = init_repo();
    let dir = repo.path();
    let mut base = numbered(30).join("\r\n");
    base.push_str("\r\nlast line without newline");
    write(dir, "crlf.txt", &base);
    commit_all(dir, "base");
    let edited = base
        .replace("line 4\r\n", "line 4 EDIT\r\n")
        .replace("last line without newline", "last line EDITED");
    write(dir, "crlf.txt", &edited);
    assert_eq!(hunks(dir, "crlf.txt").len(), 2);

    let (result, _) = stage_hunk(dir, "crlf.txt", 0).await;
    assert_eq!(result.unwrap(), StageOutcome::HunkApplied);
    let expected = base.replace("line 4\r\n", "line 4 EDIT\r\n");
    assert_eq!(git(dir, &["show", ":crlf.txt"]), expected.as_bytes());

    let (result, _) = stage_hunk(dir, "crlf.txt", 0).await;
    assert_eq!(result.unwrap(), StageOutcome::HunkApplied);
    assert_eq!(git(dir, &["show", ":crlf.txt"]), edited.as_bytes());
}

#[tokio::test]
async fn a_block_whose_file_changed_since_it_was_displayed_is_not_applied() {
    let (repo, _) = three_block_repo();
    let dir = repo.path();
    let (info, snapshot) = open(dir).await;
    let hunks_before = hunks(dir, "f.txt");
    let spec = hunks_before[1];
    let before = git(dir, &["ls-files", "--stage", "-z"]);

    let mut edited = numbered(40);
    edited[2] = "line 3 CHANGED".into();
    edited[19] = "line 20 CHANGED DIFFERENTLY".into();
    edited[37] = "line 38 CHANGED".into();
    write(dir, "f.txt", &text(&edited));
    assert_eq!(
        hunks(dir, "f.txt"),
        vec![hunks_before[0], spec, hunks_before[2]],
        "same block layout: only the content of the middle block differs"
    );
    let result = Git::new()
        .stage_hunk(&info, &target(&snapshot, "f.txt"), &spec, &Cancel::new())
        .await
        .unwrap();
    assert_eq!(
        result,
        StageOutcome::Stale {
            paths: vec![PathBuf::from("f.txt")]
        }
    );
    assert_eq!(git(dir, &["ls-files", "--stage", "-z"]), before);
}

#[tokio::test]
async fn a_block_that_no_longer_matches_the_current_diff_is_stale() {
    let (repo, _) = three_block_repo();
    let dir = repo.path();
    let (info, snapshot) = open(dir).await;
    let mut spec = hunks(dir, "f.txt")[1];
    spec.old_start += 1;
    let before = git(dir, &["ls-files", "--stage", "-z"]);
    let result = Git::new()
        .stage_hunk(&info, &target(&snapshot, "f.txt"), &spec, &Cancel::new())
        .await
        .unwrap();
    assert!(matches!(result, StageOutcome::Stale { .. }), "{result:?}");
    let missing = HunkSpec {
        index: 9,
        ..hunks(dir, "f.txt")[0]
    };
    let result = Git::new()
        .stage_hunk(&info, &target(&snapshot, "f.txt"), &missing, &Cancel::new())
        .await
        .unwrap();
    assert!(matches!(result, StageOutcome::Stale { .. }));
    assert_eq!(git(dir, &["ls-files", "--stage", "-z"]), before);
}

#[tokio::test]
async fn unusual_file_names_work_and_other_files_are_never_touched() {
    let repo = init_repo();
    let dir = repo.path();
    let name = "dossier é/new\nline [x].txt";
    let base = numbered(30);
    write(dir, name, &text(&base));
    write(dir, "bystander.txt", "same\n");
    commit_all(dir, "base");
    let mut edited = base.clone();
    edited[1] = "line 2 EDIT".into();
    edited[27] = "line 28 EDIT".into();
    write(dir, name, &text(&edited));
    write(dir, "bystander.txt", "changed\n");
    let bystander = git(dir, &["ls-files", "--stage", "-z", "--", "bystander.txt"]);

    let (result, _) = stage_hunk(dir, name, 1).await;
    assert_eq!(result.unwrap(), StageOutcome::HunkApplied);
    let mut expected = base;
    expected[27] = "line 28 EDIT".into();
    assert_eq!(
        String::from_utf8(git(dir, &["show", &format!(":{name}")])).unwrap(),
        text(&expected)
    );
    assert_eq!(
        git(dir, &["ls-files", "--stage", "-z", "--", "bystander.txt"]),
        bystander
    );
}

#[tokio::test]
async fn files_that_cannot_be_split_are_refused_with_a_reason_and_nothing_changes() {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "plain.txt", &text(&numbered(20)));
    write(dir, "exec.sh", "echo 1\necho 2\n");
    write(dir, "gone.txt", "bye\n");
    commit_all(dir, "base");
    write(dir, "untracked.txt", "new\n");
    write(dir, "exec.sh", "echo 1 changed\necho 2\n");
    std::fs::set_permissions(
        dir.join("exec.sh"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    std::fs::remove_file(dir.join("gone.txt")).unwrap();
    let before = git(dir, &["ls-files", "--stage", "-z"]);
    let (info, snapshot) = open(dir).await;
    let spec = HunkSpec {
        index: 0,
        old_start: 1,
        old_len: 1,
        new_start: 1,
        new_len: 1,
    };
    for path in ["untracked.txt", "exec.sh", "gone.txt"] {
        let result = Git::new()
            .stage_hunk(&info, &target(&snapshot, path), &spec, &Cancel::new())
            .await
            .unwrap();
        assert_eq!(
            result,
            StageOutcome::Refused {
                path: PathBuf::from(path),
                reason: Refusal::NotPartial
            },
            "{path}"
        );
    }
    assert_eq!(git(dir, &["ls-files", "--stage", "-z"]), before);
}

#[tokio::test]
async fn a_conflict_and_a_repository_with_filters_are_refused() {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "a.txt", &text(&numbered(10)));
    commit_all(dir, "base");
    git(dir, &["checkout", "-q", "-b", "side"]);
    write(dir, "a.txt", "side\n");
    commit_all(dir, "side");
    git(dir, &["checkout", "-q", "main"]);
    write(dir, "a.txt", "main\n");
    commit_all(dir, "main");
    let _ = git_output(dir, &["merge", "side"]);
    let (info, snapshot) = open(dir).await;
    let spec = HunkSpec {
        index: 0,
        old_start: 1,
        old_len: 1,
        new_start: 1,
        new_len: 1,
    };
    let result = Git::new()
        .stage_hunk(&info, &target(&snapshot, "a.txt"), &spec, &Cancel::new())
        .await
        .unwrap();
    assert_eq!(
        result,
        StageOutcome::Refused {
            path: PathBuf::from("a.txt"),
            reason: Refusal::Conflict
        }
    );

    let (repo, _) = three_block_repo();
    git(repo.path(), &["config", "filter.up.clean", "cat"]);
    let (result, _) = stage_hunk(repo.path(), "f.txt", 0).await;
    assert_eq!(
        result.unwrap(),
        StageOutcome::Refused {
            path: PathBuf::from("f.txt"),
            reason: Refusal::FilterConfigured
        }
    );
}
