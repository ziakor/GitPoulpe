mod common;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use common::*;
use gitpoulpe_core::{
    Generation, HunkLines, HunkSpec, LineSelection, Refusal, SessionInfo, StageOutcome,
    StageTarget, StatusSnapshot,
};
use gitpoulpe_git::{Cancel, Git};

struct Hunk {
    spec: HunkSpec,
    body: Vec<(char, String)>,
}

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
        index_id: snapshot.index_ids.get(&entry.path).cloned(),
        entry,
    }
}

fn cached_hunks(dir: &Path, path: &str) -> Vec<Hunk> {
    let diff = git(dir, &["diff", "--cached", "--unified=3", "--", path]);
    let text = String::from_utf8_lossy(&diff).into_owned();
    let mut hunks: Vec<Hunk> = Vec::new();
    for line in text.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("@@ -") {
            let mut parts = rest.split(' ');
            let range = |part: &str| {
                let part = part.trim_start_matches('+');
                match part.split_once(',') {
                    Some((a, b)) => (a.parse().unwrap(), b.parse().unwrap()),
                    None => (part.parse().unwrap(), 1u32),
                }
            };
            let (old_start, old_len) = range(parts.next().unwrap());
            let (new_start, new_len) = range(parts.next().unwrap());
            hunks.push(Hunk {
                spec: HunkSpec {
                    index: hunks.len(),
                    old_start,
                    old_len,
                    new_start,
                    new_len,
                },
                body: Vec::new(),
            });
        } else if let Some(current) = hunks.last_mut() {
            let kind = line.chars().next().unwrap();
            current
                .body
                .push((kind, line[1..].trim_end_matches('\n').to_owned()));
        }
    }
    hunks
}

fn lines(text: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(text)
        .split_inclusive('\n')
        .map(|l| l.trim_end_matches('\n').to_owned())
        .collect()
}

fn joined(lines: &[String]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn numbered(count: usize) -> Vec<String> {
    (1..=count).map(|n| format!("line {n}")).collect()
}

fn index_entries(dir: &Path) -> Vec<u8> {
    git(dir, &["ls-files", "--stage", "-z"])
}

fn worktree_bytes(dir: &Path, name: &str) -> Vec<u8> {
    std::fs::read(dir.join(name)).unwrap()
}

#[tokio::test]
async fn unstaging_a_file_keeps_all_its_content_in_the_working_directory() {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "a.txt", "one\ntwo\n");
    write(dir, "b.txt", "bee\n");
    commit_all(dir, "base");
    write(dir, "a.txt", "one\ntwo staged\n");
    write(dir, "b.txt", "bee staged\n");
    git(dir, &["add", "a.txt", "b.txt"]);
    write(dir, "a.txt", "one\ntwo staged\nextra local work\n");
    let worktree_a = worktree_bytes(dir, "a.txt");
    let staged_b = git(dir, &["ls-files", "--stage", "-z", "--", "b.txt"]);

    let (info, snapshot) = open(dir).await;
    let outcome = Git::new()
        .unstage(&info, &[target(&snapshot, "a.txt")], &Cancel::new())
        .await
        .unwrap();
    assert_eq!(outcome, StageOutcome::Unstaged { files: 1 });

    assert_eq!(
        git(dir, &["show", ":a.txt"]),
        b"one\ntwo\n",
        "the index is back to HEAD"
    );
    assert_eq!(
        worktree_bytes(dir, "a.txt"),
        worktree_a,
        "nothing local is lost"
    );
    assert_eq!(
        git(dir, &["ls-files", "--stage", "-z", "--", "b.txt"]),
        staged_b,
        "the other staged file is preserved"
    );
}

#[tokio::test]
async fn unstaging_works_before_the_first_commit() {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "first.txt", "1\n");
    write(dir, "second.txt", "2\n");
    git(dir, &["add", "first.txt", "second.txt"]);
    let (info, snapshot) = open(dir).await;
    assert!(!info.has_commits);

    let outcome = Git::new()
        .unstage(&info, &[target(&snapshot, "first.txt")], &Cancel::new())
        .await
        .unwrap();
    assert_eq!(outcome, StageOutcome::Unstaged { files: 1 });
    let staged = sorted(nul_paths(&git(dir, &["ls-files", "-z"])));
    assert_eq!(staged, vec![PathBuf::from("second.txt")]);
    assert_eq!(worktree_bytes(dir, "first.txt"), b"1\n");
}

#[tokio::test]
async fn a_staged_rename_is_undone_as_a_whole() {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "old name.txt", &joined(&numbered(20)));
    commit_all(dir, "base");
    git(dir, &["mv", "old name.txt", "new name.txt"]);
    let (info, snapshot) = open(dir).await;
    let entry = target(&snapshot, "new name.txt");
    assert!(entry.entry.original_path.is_some(), "git reports a rename");

    let outcome = Git::new()
        .unstage(&info, &[entry], &Cancel::new())
        .await
        .unwrap();
    assert_eq!(outcome, StageOutcome::Unstaged { files: 1 });
    assert_eq!(
        sorted(nul_paths(&git(dir, &["ls-files", "-z"]))),
        vec![PathBuf::from("old name.txt")],
        "the old path is back in the index and the new one is gone"
    );
    assert!(
        dir.join("new name.txt").exists(),
        "the renamed file stays on disk"
    );
}

#[tokio::test]
async fn unusual_names_conflicts_and_files_with_nothing_staged_are_handled() {
    let repo = init_repo();
    let dir = repo.path();
    let odd: Vec<OsString> = vec![
        "star*.txt".into(),
        "-dash.txt".into(),
        OsString::from_vec(b"bytes-\xff.txt".to_vec()),
        OsString::from_vec(b"new\nline.txt".to_vec()),
    ];
    for name in &odd {
        std::fs::write(dir.join(name), "x\n").unwrap();
    }
    std::fs::write(dir.join("starX.txt"), "look-alike\n").unwrap();
    git(dir, &["add", "-A"]);
    let (info, snapshot) = open(dir).await;
    // non-UTF-8 names cannot be looked up through a &str, so build those targets by entry
    let by_entry: Vec<StageTarget> = snapshot
        .entries
        .iter()
        .filter(|e| odd.iter().any(|n| e.path.as_os_str() == n.as_os_str()))
        .map(|entry| StageTarget {
            stamp: None,
            index_id: snapshot.index_ids.get(&entry.path).cloned(),
            entry: entry.clone(),
        })
        .collect();
    assert_eq!(by_entry.len(), 4);
    let outcome = Git::new()
        .unstage(&info, &by_entry, &Cancel::new())
        .await
        .unwrap();
    assert_eq!(outcome, StageOutcome::Unstaged { files: 4 });
    assert_eq!(
        sorted(nul_paths(&git(dir, &["ls-files", "-z"]))),
        vec![PathBuf::from("starX.txt")],
        "only the literal names left the index"
    );

    let (info, snapshot) = open(dir).await;
    let before = index_entries(dir);
    let nothing = Git::new()
        .unstage(&info, &[target(&snapshot, "starX.txt")], &Cancel::new())
        .await
        .unwrap();
    assert!(matches!(nothing, StageOutcome::Unstaged { files: 1 }));
    let (info, snapshot) = open(dir).await;
    let again = StageTarget {
        entry: snapshot
            .entries
            .iter()
            .find(|e| e.path == Path::new("star*.txt"))
            .unwrap()
            .clone(),
        stamp: None,
        index_id: None,
    };
    let outcome = Git::new()
        .unstage(&info, &[again], &Cancel::new())
        .await
        .unwrap();
    assert!(
        matches!(
            outcome,
            StageOutcome::Refused {
                reason: Refusal::NothingToUnstage,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_ne!(before, Vec::new());
}

#[tokio::test]
async fn a_conflict_is_refused() {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "a.txt", "base\n");
    commit_all(dir, "base");
    git(dir, &["checkout", "-q", "-b", "side"]);
    write(dir, "a.txt", "side\n");
    commit_all(dir, "side");
    git(dir, &["checkout", "-q", "main"]);
    write(dir, "a.txt", "main\n");
    commit_all(dir, "main");
    let _ = git_output(dir, &["merge", "side"]);
    let (info, snapshot) = open(dir).await;
    let before = index_entries(dir);
    let outcome = Git::new()
        .unstage(&info, &[target(&snapshot, "a.txt")], &Cancel::new())
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        StageOutcome::Refused {
            reason: Refusal::Conflict,
            ..
        }
    ));
    assert_eq!(index_entries(dir), before);
}

#[tokio::test]
async fn an_index_that_changed_after_it_was_displayed_is_refused() {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "a.txt", "one\ntwo\n");
    commit_all(dir, "base");
    write(dir, "a.txt", "one\ntwo changed\n");
    git(dir, &["add", "a.txt"]);
    let (info, snapshot) = open(dir).await;
    let seen = target(&snapshot, "a.txt");

    write(dir, "a.txt", "one\ntwo changed\nthree added later\n");
    git(dir, &["add", "a.txt"]);
    let before = index_entries(dir);
    let outcome = Git::new()
        .unstage(&info, &[seen], &Cancel::new())
        .await
        .unwrap();
    assert!(matches!(outcome, StageOutcome::Stale { .. }), "{outcome:?}");
    assert_eq!(index_entries(dir), before);
}

fn three_block_staged() -> (tempfile::TempDir, Vec<String>) {
    let repo = init_repo();
    let base = numbered(60);
    write(repo.path(), "f.txt", &joined(&base));
    commit_all(repo.path(), "base");
    let mut edited = base.clone();
    edited[3] = "line 4 CHANGED".into();
    edited.insert(30, "inserted".into());
    edited.drain(48..51);
    write(repo.path(), "f.txt", &joined(&edited));
    git(repo.path(), &["add", "f.txt"]);
    (repo, base)
}

fn oracle(base: &[String], hunks: &[Hunk], picked: &BTreeSet<(usize, usize)>) -> Vec<String> {
    let mut out = Vec::new();
    let mut pointer = 0usize;
    for hunk in hunks {
        let upto = if hunk.spec.old_len == 0 {
            hunk.spec.old_start as usize
        } else {
            hunk.spec.old_start as usize - 1
        };
        out.extend_from_slice(&base[pointer..upto]);
        pointer = upto;
        for (offset, (kind, text)) in hunk.body.iter().enumerate() {
            let chosen = picked.contains(&(hunk.spec.index, offset));
            match kind {
                ' ' => {
                    out.push(text.clone());
                    pointer += 1;
                }
                '-' => {
                    if chosen {
                        out.push(text.clone());
                    }
                    pointer += 1;
                }
                '+' if !chosen => out.push(text.clone()),
                _ => {}
            }
        }
    }
    out.extend_from_slice(&base[pointer..]);
    out
}

fn changed(hunks: &[Hunk]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for hunk in hunks {
        for (offset, (kind, _)) in hunk.body.iter().enumerate() {
            if *kind == '+' || *kind == '-' {
                out.push((hunk.spec.index, offset));
            }
        }
    }
    out
}

fn selection(hunks: &[Hunk], picked: &BTreeSet<(usize, usize)>) -> LineSelection {
    let mut out = Vec::new();
    for hunk in hunks {
        let ranges: Vec<(u32, u32)> = picked
            .iter()
            .filter(|(h, _)| *h == hunk.spec.index)
            .map(|(_, o)| (*o as u32, *o as u32))
            .collect();
        if !ranges.is_empty() {
            out.push(HunkLines {
                hunk: hunk.spec,
                ranges,
            });
        }
    }
    LineSelection { hunks: out }
}

#[tokio::test]
async fn unstaging_one_block_leaves_the_others_staged_and_the_worktree_intact() {
    let (repo, base) = three_block_staged();
    let dir = repo.path();
    let all = cached_hunks(dir, "f.txt");
    assert_eq!(all.len(), 3);
    let worktree = worktree_bytes(dir, "f.txt");
    let (info, snapshot) = open(dir).await;

    let outcome = Git::new()
        .unstage_hunk(
            &info,
            &target(&snapshot, "f.txt"),
            &all[1].spec,
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert_eq!(outcome, StageOutcome::HunkUnstaged);

    let picked: BTreeSet<(usize, usize)> =
        changed(&all).into_iter().filter(|(h, _)| *h == 1).collect();
    assert_eq!(
        lines(&git(dir, &["show", ":f.txt"])),
        oracle(&base, &all, &picked)
    );
    assert_eq!(worktree_bytes(dir, "f.txt"), worktree);
    let still = cached_hunks(dir, "f.txt");
    assert_eq!(still.len(), 2, "the other two blocks are still staged");
}

#[tokio::test]
async fn any_random_line_selection_unstages_exactly_what_the_oracle_predicts() {
    let (repo, base) = three_block_staged();
    let dir = repo.path();
    let all = cached_hunks(dir, "f.txt");
    let every = changed(&all);
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for round in 0..100 {
        git(dir, &["add", "f.txt"]);
        let density = 1 + next() % 7;
        let picked: BTreeSet<(usize, usize)> = every
            .iter()
            .copied()
            .filter(|_| next() % 8 < density)
            .collect();
        let (info, snapshot) = open(dir).await;
        let outcome = Git::new()
            .unstage_lines(
                &info,
                &target(&snapshot, "f.txt"),
                &selection(&all, &picked),
                &Cancel::new(),
            )
            .await
            .unwrap();
        if picked.is_empty() {
            assert!(
                matches!(
                    outcome,
                    StageOutcome::Refused {
                        reason: Refusal::NothingSelected,
                        ..
                    }
                ),
                "round {round}: {outcome:?}"
            );
            continue;
        }
        assert_eq!(
            outcome,
            StageOutcome::LinesUnstaged {
                lines: picked.len()
            },
            "round {round}"
        );
        assert_eq!(
            lines(&git(dir, &["show", ":f.txt"])),
            oracle(&base, &all, &picked),
            "round {round}, picked {picked:?}"
        );
    }
}

#[tokio::test]
async fn lines_of_a_newly_added_file_can_be_unstaged_and_all_of_them_remove_the_file() {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "seed.txt", "seed\n");
    commit_all(dir, "base");
    write(dir, "new file.txt", &joined(&numbered(10)));
    git(dir, &["add", "new file.txt"]);
    let all = cached_hunks(dir, "new file.txt");
    assert_eq!(all.len(), 1);
    let worktree = worktree_bytes(dir, "new file.txt");

    let picked: BTreeSet<(usize, usize)> = [(0, 2), (0, 3), (0, 9)].into();
    let (info, snapshot) = open(dir).await;
    let outcome = Git::new()
        .unstage_lines(
            &info,
            &target(&snapshot, "new file.txt"),
            &selection(&all, &picked),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert_eq!(outcome, StageOutcome::LinesUnstaged { lines: 3 });
    let expected: Vec<String> = numbered(10)
        .into_iter()
        .enumerate()
        .filter(|(i, _)| ![2, 3, 9].contains(i))
        .map(|(_, l)| l)
        .collect();
    assert_eq!(lines(&git(dir, &["show", ":new file.txt"])), expected);
    assert_eq!(worktree_bytes(dir, "new file.txt"), worktree);

    let all = cached_hunks(dir, "new file.txt");
    let everything: BTreeSet<(usize, usize)> = changed(&all).into_iter().collect();
    let (info, snapshot) = open(dir).await;
    let outcome = Git::new()
        .unstage_lines(
            &info,
            &target(&snapshot, "new file.txt"),
            &selection(&all, &everything),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome, StageOutcome::LinesUnstaged { .. }),
        "{outcome:?}"
    );
    assert!(
        !sorted(nul_paths(&git(dir, &["ls-files", "-z"]))).contains(&PathBuf::from("new file.txt")),
        "unstaging every line unstages the new file"
    );
    assert_eq!(worktree_bytes(dir, "new file.txt"), worktree);
}

#[tokio::test]
async fn partial_unstaging_is_refused_for_deletions_and_stale_selections_change_nothing() {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "gone.txt", &joined(&numbered(10)));
    write(dir, "f.txt", &joined(&numbered(30)));
    commit_all(dir, "base");
    std::fs::remove_file(dir.join("gone.txt")).unwrap();
    let mut edited = numbered(30);
    edited[5] = "six".into();
    write(dir, "f.txt", &joined(&edited));
    git(dir, &["add", "-A"]);
    let (info, snapshot) = open(dir).await;
    let spec = HunkSpec {
        index: 0,
        old_start: 1,
        old_len: 1,
        new_start: 1,
        new_len: 1,
    };
    let outcome = Git::new()
        .unstage_hunk(&info, &target(&snapshot, "gone.txt"), &spec, &Cancel::new())
        .await
        .unwrap();
    assert!(
        matches!(
            outcome,
            StageOutcome::Refused {
                reason: Refusal::NotPartial,
                ..
            }
        ),
        "{outcome:?}"
    );

    let all = cached_hunks(dir, "f.txt");
    let seen = target(&snapshot, "f.txt");
    let mut wrong = all[0].spec;
    wrong.old_start += 1;
    let before = index_entries(dir);
    let outcome = Git::new()
        .unstage_hunk(&info, &seen, &wrong, &Cancel::new())
        .await
        .unwrap();
    assert!(matches!(outcome, StageOutcome::Stale { .. }), "{outcome:?}");
    assert_eq!(index_entries(dir), before);
}
