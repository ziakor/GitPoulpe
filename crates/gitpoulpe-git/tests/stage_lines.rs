mod common;

use std::collections::BTreeSet;
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
        .unwrap()
        .clone();
    StageTarget {
        stamp: snapshot.stamps.get(&entry.path).copied(),
        entry,
    }
}

fn hunks(dir: &Path, path: &str) -> Vec<Hunk> {
    let diff = git(dir, &["diff", "--unified=3", "--", path]);
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
                    if !chosen {
                        out.push(text.clone());
                    }
                    pointer += 1;
                }
                '+' if chosen => out.push(text.clone()),
                _ => {}
            }
        }
    }
    out.extend_from_slice(&base[pointer..]);
    out
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

fn base_lines(count: usize) -> Vec<String> {
    (1..=count).map(|n| format!("line {n}")).collect()
}

fn mixed_repo() -> (tempfile::TempDir, Vec<String>) {
    let repo = init_repo();
    let base = base_lines(80);
    write(repo.path(), "f.txt", &joined(&base));
    commit_all(repo.path(), "base");
    let mut edited = base.clone();
    edited.splice(4..5, ["five-a".to_owned(), "five-b".to_owned()]);
    edited.insert(24, "inserted".to_owned());
    edited.drain(38..42);
    edited[50] = "fifty-one changed".into();
    edited.insert(60, "late one".into());
    edited.insert(61, "late two".into());
    write(repo.path(), "f.txt", &joined(&edited));
    (repo, base)
}

fn changed_offsets(hunks: &[Hunk]) -> Vec<(usize, usize)> {
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

async fn stage(
    dir: &Path,
    picked: &BTreeSet<(usize, usize)>,
) -> Result<StageOutcome, gitpoulpe_core::WriteError> {
    let (info, snapshot) = open(dir).await;
    let hunks = hunks(dir, "f.txt");
    Git::new()
        .stage_lines(
            &info,
            &target(&snapshot, "f.txt"),
            &selection(&hunks, picked),
            &Cancel::new(),
        )
        .await
}

#[tokio::test]
async fn a_mixed_selection_of_added_and_removed_lines_matches_the_independent_oracle() {
    let (repo, base) = mixed_repo();
    let dir = repo.path();
    let all = hunks(dir, "f.txt");
    assert!(all.len() >= 4, "{} hunks", all.len());
    let changed = changed_offsets(&all);
    let picked: BTreeSet<(usize, usize)> = changed
        .iter()
        .copied()
        .enumerate()
        .filter(|(i, _)| i % 2 == 0)
        .map(|(_, c)| c)
        .collect();

    let outcome = stage(dir, &picked).await.unwrap();
    assert_eq!(
        outcome,
        StageOutcome::LinesApplied {
            lines: picked.len()
        }
    );
    assert_eq!(
        lines(&git(dir, &["show", ":f.txt"])),
        oracle(&base, &all, &picked)
    );
    assert_ne!(
        git(dir, &["show", ":f.txt"]),
        std::fs::read(dir.join("f.txt")).unwrap(),
        "the rest is still unstaged"
    );
}

#[tokio::test]
async fn any_random_selection_gives_exactly_the_content_the_oracle_predicts() {
    let (repo, base) = mixed_repo();
    let dir = repo.path();
    let all = hunks(dir, "f.txt");
    let changed = changed_offsets(&all);
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for round in 0..120 {
        git(dir, &["reset", "-q"]);
        let density = 1 + next() % 7;
        let picked: BTreeSet<(usize, usize)> = changed
            .iter()
            .copied()
            .filter(|_| next() % 8 < density)
            .collect();
        let outcome = stage(dir, &picked).await.unwrap();
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
            StageOutcome::LinesApplied {
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
async fn a_selection_made_of_ranges_covers_the_changed_lines_between_its_ends() {
    let (repo, base) = mixed_repo();
    let dir = repo.path();
    let all = hunks(dir, "f.txt");
    let last = all.last().unwrap();
    let ranges = vec![
        HunkLines {
            hunk: all[0].spec,
            ranges: vec![(0, 1_000)],
        },
        HunkLines {
            hunk: last.spec,
            ranges: vec![(0, 1_000)],
        },
    ];
    let (info, snapshot) = open(dir).await;
    let outcome = Git::new()
        .stage_lines(
            &info,
            &target(&snapshot, "f.txt"),
            &LineSelection { hunks: ranges },
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert!(matches!(outcome, StageOutcome::LinesApplied { .. }));
    let picked: BTreeSet<(usize, usize)> = changed_offsets(&all)
        .into_iter()
        .filter(|(h, _)| *h == 0 || *h == last.spec.index)
        .collect();
    assert_eq!(
        lines(&git(dir, &["show", ":f.txt"])),
        oracle(&base, &all, &picked),
        "context lines inside a range are not 'staged' and nothing else moves"
    );
}

#[tokio::test]
async fn a_selection_of_context_only_or_nothing_is_refused_and_the_index_is_untouched() {
    let (repo, _) = mixed_repo();
    let dir = repo.path();
    let before = git(dir, &["ls-files", "--stage", "-z"]);
    let all = hunks(dir, "f.txt");
    let context_offset = all[0].body.iter().position(|(k, _)| *k == ' ').unwrap();
    let picked: BTreeSet<(usize, usize)> = [(0, context_offset)].into();
    let outcome = stage(dir, &picked).await.unwrap();
    assert!(matches!(
        outcome,
        StageOutcome::Refused {
            reason: Refusal::NothingSelected,
            ..
        }
    ));
    assert_eq!(git(dir, &["ls-files", "--stage", "-z"]), before);
}

#[tokio::test]
async fn a_partial_selection_next_to_a_missing_final_newline_is_refused_but_a_full_one_works() {
    let repo = init_repo();
    let dir = repo.path();
    let base = "alpha\nbeta\ngamma\ndelta\nlast";
    write(dir, "f.txt", base);
    commit_all(dir, "base");
    write(
        dir,
        "f.txt",
        "alpha CHANGED\nbeta\ngamma\ndelta\nlast CHANGED",
    );
    let all = hunks(dir, "f.txt");
    let changed = changed_offsets(&all);
    assert!(changed.len() >= 4);

    let partial: BTreeSet<(usize, usize)> = [changed[0]].into();
    let outcome = stage(dir, &partial).await.unwrap();
    assert!(
        matches!(
            outcome,
            StageOutcome::Refused {
                reason: Refusal::Incompatible,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(git(dir, &["show", ":f.txt"]), base.as_bytes());

    let full: BTreeSet<(usize, usize)> = changed.iter().copied().collect();
    let outcome = stage(dir, &full).await.unwrap();
    assert!(
        matches!(outcome, StageOutcome::LinesApplied { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        git(dir, &["show", ":f.txt"]),
        std::fs::read(dir.join("f.txt")).unwrap()
    );
}

#[tokio::test]
async fn crlf_files_stage_selected_lines_without_touching_line_endings() {
    let repo = init_repo();
    let dir = repo.path();
    let base: String = (1..=30).map(|n| format!("row {n}\r\n")).collect();
    write(dir, "f.txt", &base);
    commit_all(dir, "base");
    let edited = base
        .replace("row 5\r\n", "row 5 EDIT\r\nrow 5b\r\n")
        .replace("row 25\r\n", "");
    write(dir, "f.txt", &edited);
    let (info, snapshot) = open(dir).await;
    let parsed = {
        let diff = git(dir, &["diff", "--unified=3", "--", "f.txt"]);
        String::from_utf8_lossy(&diff)
            .lines()
            .filter(|l| l.starts_with("@@"))
            .count()
    };
    assert_eq!(parsed, 2);
    let all = hunks(dir, "f.txt");
    let added: Vec<(usize, usize)> = changed_offsets(&all)
        .into_iter()
        .filter(|(h, o)| all[*h].body[*o].0 == '+' && all[*h].body[*o].1.contains("5b"))
        .collect();
    assert_eq!(added.len(), 1);
    let picked: BTreeSet<(usize, usize)> = added.into_iter().collect();
    let outcome = Git::new()
        .stage_lines(
            &info,
            &target(&snapshot, "f.txt"),
            &selection(&all, &picked),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome, StageOutcome::LinesApplied { lines: 1 }),
        "{outcome:?}"
    );
    let staged = git(dir, &["show", ":f.txt"]);
    let expected = base.replacen("row 5\r\n", "row 5\r\nrow 5b\r\n", 1);
    assert_eq!(String::from_utf8(staged).unwrap(), expected);
}

#[tokio::test]
async fn a_stale_selection_is_refused_before_anything_is_written() {
    let (repo, _) = mixed_repo();
    let dir = repo.path();
    let (info, snapshot) = open(dir).await;
    let all = hunks(dir, "f.txt");
    let picked: BTreeSet<(usize, usize)> = changed_offsets(&all).into_iter().take(2).collect();
    let chosen = selection(&all, &picked);
    let before = git(dir, &["ls-files", "--stage", "-z"]);

    let edited = String::from_utf8(std::fs::read(dir.join("f.txt")).unwrap())
        .unwrap()
        .replace("fifty-one changed", "fifty-one rewritten");
    write(dir, "f.txt", &edited);
    let outcome = Git::new()
        .stage_lines(&info, &target(&snapshot, "f.txt"), &chosen, &Cancel::new())
        .await
        .unwrap();
    assert_eq!(
        outcome,
        StageOutcome::Stale {
            paths: vec![PathBuf::from("f.txt")]
        }
    );
    assert_eq!(git(dir, &["ls-files", "--stage", "-z"]), before);

    let (info, snapshot) = open(dir).await;
    let mut wrong = chosen.clone();
    wrong.hunks[0].hunk.old_start += 1;
    let outcome = Git::new()
        .stage_lines(&info, &target(&snapshot, "f.txt"), &wrong, &Cancel::new())
        .await
        .unwrap();
    assert!(matches!(outcome, StageOutcome::Stale { .. }));
    assert_eq!(git(dir, &["ls-files", "--stage", "-z"]), before);
}

#[tokio::test]
async fn lines_far_apart_in_a_fifty_thousand_line_file_are_staged_exactly() {
    let repo = init_repo();
    let dir = repo.path();
    let base = base_lines(50_000);
    write(dir, "f.txt", &joined(&base));
    commit_all(dir, "base");
    let mut edited = base.clone();
    edited[9] = "line 10 EDIT".into();
    edited.insert(25_000, "inserted at the middle".into());
    edited.drain(49_980..49_983);
    write(dir, "f.txt", &joined(&edited));
    let all = hunks(dir, "f.txt");
    assert_eq!(all.len(), 3);
    let picked: BTreeSet<(usize, usize)> = changed_offsets(&all)
        .into_iter()
        .filter(|(h, _)| *h != 1)
        .collect();
    let outcome = stage(dir, &picked).await.unwrap();
    assert!(
        matches!(outcome, StageOutcome::LinesApplied { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        lines(&git(dir, &["show", ":f.txt"])),
        oracle(&base, &all, &picked)
    );
}
