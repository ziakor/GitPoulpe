mod common;

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use common::{commit_all, git as run_git, git_output, init_repo, private_tempdir, write};
use gitpoulpe_core::{
    ChangeEntry, CommitRange, DiffLine, DiffLineKind, DocumentKind, EntryKind, Generation,
    SessionInfo, StatusCode,
};
use gitpoulpe_git::{CacheConfig, Cancel, DiffCache, DiffDocument, DiffReader, Git, InspectError};

struct Reader {
    _cache: tempfile::TempDir,
    reader: DiffReader,
    info: SessionInfo,
    generation: Generation,
}

async fn reader(dir: &Path) -> Reader {
    let cache = private_tempdir();
    let git = Git::new();
    let reader = DiffReader::new(
        git.clone(),
        DiffCache::open(CacheConfig::new(cache.path())).unwrap(),
    );
    let info = git.open(dir, &Cancel::new()).await.unwrap();
    Reader {
        _cache: cache,
        reader,
        info,
        generation: Generation::default().next(),
    }
}

async fn all_lines(doc: &DiffDocument) -> Vec<DiffLine> {
    let cancel = Cancel::new();
    let mut lines = Vec::new();
    loop {
        let data = doc
            .read_range(lines.len() as u64..lines.len() as u64 + 700, &cancel)
            .await
            .unwrap();
        let done = data.lines.is_empty()
            || data.total_lines == Some(lines.len() as u64 + data.lines.len() as u64);
        lines.extend(data.lines);
        if done {
            return lines;
        }
    }
}

fn rev(dir: &Path, name: &str) -> String {
    String::from_utf8(run_git(dir, &["rev-parse", name]))
        .unwrap()
        .trim()
        .to_owned()
}

fn changes_of(lines: &[DiffLine]) -> Vec<(char, Vec<u8>)> {
    lines
        .iter()
        .filter_map(|l| match l.kind {
            DiffLineKind::Added => Some(('+', l.text.clone())),
            DiffLineKind::Removed => Some(('-', l.text.clone())),
            _ => None,
        })
        .collect()
}

fn git_changes(dir: &Path, base: &str, oid: &str, entry: &ChangeEntry) -> Vec<(char, Vec<u8>)> {
    let mut args: Vec<OsString> = ["diff", "-M", "--unified=3", base, oid, "--"]
        .map(OsString::from)
        .to_vec();
    let literal = |p: &PathBuf| {
        let mut spec = b":(literal)".to_vec();
        spec.extend_from_slice(p.as_os_str().as_encoded_bytes());
        OsString::from_vec(spec)
    };
    args.push(literal(&entry.path));
    if let Some(original) = &entry.original_path {
        args.push(literal(original));
    }
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args(&args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    let mut in_hunk = false;
    let mut out = Vec::new();
    for line in output.stdout.split(|b| *b == b'\n') {
        if line.starts_with(b"@@") {
            in_hunk = true;
            continue;
        }
        if !in_hunk || line.is_empty() {
            continue;
        }
        match line[0] {
            b'+' => out.push(('+', line[1..].to_vec())),
            b'-' => out.push(('-', line[1..].to_vec())),
            _ => {}
        }
    }
    out
}

fn status_of(entries: &[ChangeEntry], path: &str) -> StatusCode {
    entries
        .iter()
        .find(|e| e.path == Path::new(path))
        .unwrap_or_else(|| {
            panic!(
                "{path} is not listed: {:?}",
                entries.iter().map(|e| &e.path).collect::<Vec<_>>()
            )
        })
        .index
}

fn history() -> (tempfile::TempDir, [String; 4]) {
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "a.txt", "one\ntwo\nthree\n");
    write(dir, "dir with space/b.txt", "bee\n");
    write(
        dir,
        "old name.txt",
        &(1..=30).map(|n| format!("line {n}\n")).collect::<String>(),
    );
    commit_all(dir, "initial commit");
    write(dir, "a.txt", "one\ntwo changed\nthree\nfour\n");
    write(dir, "c.txt", "new file\n");
    std::fs::remove_file(dir.join("dir with space/b.txt")).unwrap();
    std::fs::rename(dir.join("old name.txt"), dir.join("new name.txt")).unwrap();
    write(
        dir,
        "new name.txt",
        &(1..=30)
            .map(|n| {
                if n == 7 {
                    "line 7 edited\n".to_owned()
                } else {
                    format!("line {n}\n")
                }
            })
            .collect::<String>(),
    );
    commit_all(dir, "ordinary commit\n\nwith a body\nand two lines");
    std::fs::write(dir.join("blob.bin"), b"\0\x01\x02binary").unwrap();
    std::fs::write(
        dir.join(OsString::from_vec(b"bytes-\xff.txt".to_vec())),
        "raw\n",
    )
    .unwrap();
    write(dir, "multi\nline.txt", "newline in the name\n");
    write(dir, "star*.txt", "glob\n");
    commit_all(dir, "special paths");
    run_git(dir, &["checkout", "-q", "-b", "side", "HEAD~1"]);
    write(dir, "side.txt", "side change\n");
    write(dir, "a.txt", "one\ntwo changed\nthree\nfour\nfrom side\n");
    commit_all(dir, "side work");
    run_git(dir, &["checkout", "-q", "main"]);
    write(dir, "main-only.txt", "main change\n");
    commit_all(dir, "main work");
    let merged = git_output(dir, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    assert!(
        merged.status.success(),
        "{}",
        String::from_utf8_lossy(&merged.stderr)
    );
    let ids = [
        rev(dir, "HEAD~4"),
        rev(dir, "HEAD~3"),
        rev(dir, "HEAD~2"),
        rev(dir, "HEAD"),
    ];
    (repo, ids)
}

#[tokio::test]
async fn the_detail_of_a_commit_matches_what_git_reports() {
    let (repo, ids) = history();
    let dir = repo.path();
    let git = Git::new();
    let info = git.open(dir, &Cancel::new()).await.unwrap();

    let detail = git.commit_detail(&info, &ids[1]).await.unwrap();
    assert_eq!(detail.commit.oid, ids[1]);
    assert_eq!(detail.commit.parents, vec![ids[0].clone()]);
    assert_eq!(
        detail.message,
        "ordinary commit\n\nwith a body\nand two lines"
    );
    assert_eq!(detail.commit.subject, "ordinary commit");
    assert_eq!(
        (
            detail.commit.author.as_str(),
            detail.author_email.as_str(),
            detail.committer.as_str()
        ),
        ("t", "t@example.com", "t")
    );
    let git_time =
        String::from_utf8(run_git(dir, &["log", "-1", "--format=%at", &ids[1]])).unwrap();
    assert_eq!(detail.commit.timestamp.to_string(), git_time.trim());

    let root = git.commit_detail(&info, &ids[0]).await.unwrap();
    assert!(
        root.commit.parents.is_empty(),
        "the first commit has no parent"
    );
    let merge = git.commit_detail(&info, &ids[3]).await.unwrap();
    assert_eq!(merge.commit.parents.len(), 2);
    assert!(
        merge.commit.refs.iter().any(|r| r == "main"),
        "{:?}",
        merge.commit.refs
    );

    for bad in [
        "",
        "abc",
        "--help",
        "zzzzzzzz",
        "HEAD",
        "abcd; rm -rf /",
        "../../etc",
    ] {
        assert_eq!(
            git.commit_detail(&info, bad).await.unwrap_err(),
            InspectError::InvalidId,
            "{bad:?}"
        );
    }
    assert_eq!(
        git.commit_detail(&info, &"0".repeat(40)).await.unwrap_err(),
        InspectError::NotACommit
    );
}

#[tokio::test]
async fn the_first_commit_ordinary_and_special_commits_list_exactly_their_changes_with_diffs_equal_to_gits()
 {
    let (repo, ids) = history();
    let dir = repo.path();
    let git = Git::new();
    let r = reader(dir).await;
    let empty = git.empty_tree(&r.info).await.unwrap();
    assert_eq!(
        empty,
        String::from_utf8(run_git(dir, &["hash-object", "-t", "tree", "/dev/null"]))
            .unwrap()
            .trim()
    );

    let first = git.commit_files(&r.info, &ids[0], &empty).await.unwrap();
    assert_eq!(first.len(), 3);
    assert!(
        first.iter().all(|e| e.index == StatusCode::Added),
        "the first commit is inspectable: everything is new"
    );

    let ordinary = git.commit_files(&r.info, &ids[1], &ids[0]).await.unwrap();
    assert_eq!(status_of(&ordinary, "a.txt"), StatusCode::Modified);
    assert_eq!(status_of(&ordinary, "c.txt"), StatusCode::Added);
    assert_eq!(
        status_of(&ordinary, "dir with space/b.txt"),
        StatusCode::Deleted
    );
    let renamed = ordinary
        .iter()
        .find(|e| e.path == Path::new("new name.txt"))
        .expect("the rename is detected");
    assert_eq!(renamed.kind, EntryKind::Renamed);
    assert_eq!(
        renamed.original_path.as_deref(),
        Some(Path::new("old name.txt"))
    );

    let special = git.commit_files(&r.info, &ids[2], &ids[1]).await.unwrap();
    let names: Vec<PathBuf> = special.iter().map(|e| e.path.clone()).collect();
    assert!(names.contains(&PathBuf::from("multi\nline.txt")));
    assert!(names.contains(&PathBuf::from("star*.txt")));
    assert!(names.contains(&PathBuf::from(OsString::from_vec(
        b"bytes-\xff.txt".to_vec()
    ))));

    for (oid, base, entries) in [
        (&ids[0], &empty, &first),
        (&ids[1], &ids[0], &ordinary),
        (&ids[2], &ids[1], &special),
    ] {
        let range = CommitRange {
            oid: oid.clone(),
            base: base.clone(),
        };
        for entry in entries.iter() {
            let doc = r
                .reader
                .open_commit_document(&r.info, r.generation, &range, entry)
                .unwrap();
            let meta = doc.wait_finished(&Cancel::new()).await.unwrap();
            if entry.path == Path::new("blob.bin") {
                assert_eq!(
                    meta.kind,
                    Some(DocumentKind::Binary),
                    "binary files are announced, not shown"
                );
                continue;
            }
            let lines = all_lines(&doc).await;
            assert_eq!(
                changes_of(&lines),
                git_changes(dir, base, oid, entry),
                "{:?} in {oid}",
                entry.path
            );
        }
    }
}

#[tokio::test]
async fn a_merge_is_compared_with_the_parent_the_user_picks() {
    let (repo, ids) = history();
    let dir = repo.path();
    let git = Git::new();
    let r = reader(dir).await;
    let detail = git.commit_detail(&r.info, &ids[3]).await.unwrap();
    let [first_parent, second_parent] = [
        detail.commit.parents[0].clone(),
        detail.commit.parents[1].clone(),
    ];

    let against_first = git
        .commit_files(&r.info, &ids[3], &first_parent)
        .await
        .unwrap();
    let against_second = git
        .commit_files(&r.info, &ids[3], &second_parent)
        .await
        .unwrap();
    let expected = |base: &str| {
        let out = run_git(
            dir,
            &["diff-tree", "-r", "-z", "--name-only", base, &ids[3]],
        );
        let mut names: Vec<String> = out
            .split(|b| *b == 0)
            .filter(|n| !n.is_empty())
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .collect();
        names.sort();
        names
    };
    let names = |entries: &[ChangeEntry]| {
        let mut names: Vec<String> = entries
            .iter()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    assert_eq!(names(&against_first), expected(&first_parent));
    assert_eq!(names(&against_second), expected(&second_parent));
    assert_ne!(
        names(&against_first),
        names(&against_second),
        "the two comparisons differ"
    );
    assert!(names(&against_first).contains(&"side.txt".to_owned()));
    assert!(names(&against_second).contains(&"main-only.txt".to_owned()));

    let range = CommitRange {
        oid: ids[3].clone(),
        base: second_parent.clone(),
    };
    for entry in &against_second {
        let doc = r
            .reader
            .open_commit_document(&r.info, r.generation, &range, entry)
            .unwrap();
        if doc.wait_finished(&Cancel::new()).await.unwrap().kind == Some(DocumentKind::Binary) {
            continue;
        }
        assert_eq!(
            changes_of(&all_lines(&doc).await),
            git_changes(dir, &second_parent, &ids[3], entry)
        );
    }
}
