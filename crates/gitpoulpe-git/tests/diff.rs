mod common;

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::*;
use gitpoulpe_core::{
    BuildState, ChangeEntry, DiffLine, DiffLineKind, DiffSide, DocumentKind, EntryKind, Generation,
    RangeError, SessionInfo, StatusCode, StatusSnapshot,
};
use gitpoulpe_git::{CacheConfig, Cancel, DiffCache, DiffDocument, DiffError, DiffReader, Git};
use tempfile::TempDir;

struct Fixture {
    _repo: TempDir,
    cache_dir: TempDir,
    reader: DiffReader,
    info: SessionInfo,
    status: StatusSnapshot,
    generation: Generation,
}

impl Fixture {
    async fn new(repo: TempDir) -> Self {
        Self::with_budgets(repo, None).await
    }

    async fn with_budgets(repo: TempDir, budgets: Option<(u64, u64)>) -> Self {
        let root = repo.path().to_path_buf();
        Self::build(repo, root, budgets).await
    }

    async fn build(repo: TempDir, root: PathBuf, budgets: Option<(u64, u64)>) -> Self {
        let cache_dir = tempfile::tempdir().unwrap();
        let mut config = CacheConfig::new(cache_dir.path());
        if let Some((memory, disk)) = budgets {
            config.memory_bytes = memory;
            config.disk_bytes = disk;
        }
        let git = Git::new();
        let reader = DiffReader::new(git.clone(), DiffCache::open(config).unwrap());
        let cancel = Cancel::new();
        let info = git.open(&root, &cancel).await.unwrap();
        let generation = Generation::default().next();
        let status = git.read_status(&info, generation, &cancel).await.unwrap();
        Self {
            _repo: repo,
            cache_dir,
            reader,
            info,
            status,
            generation,
        }
    }

    async fn refresh(&mut self) {
        self.generation = self.generation.next();
        self.status = Git::new()
            .read_status(&self.info, self.generation, &Cancel::new())
            .await
            .unwrap();
    }

    fn entry(&self, path: impl AsRef<Path>) -> ChangeEntry {
        self.status
            .entries
            .iter()
            .find(|e| e.path == path.as_ref())
            .unwrap_or_else(|| panic!("no entry for {:?}", path.as_ref()))
            .clone()
    }

    fn doc(&self, side: DiffSide, path: impl AsRef<Path>) -> DiffDocument {
        self.reader
            .open_document(&self.info, self.generation, side, &self.entry(path))
            .unwrap()
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

fn render(lines: &[DiffLine]) -> Vec<u8> {
    let mut out = Vec::new();
    for line in lines {
        match line.kind {
            DiffLineKind::HunkHeader => {}
            DiffLineKind::Context => out.push(b' '),
            DiffLineKind::Added => out.push(b'+'),
            DiffLineKind::Removed => out.push(b'-'),
            DiffLineKind::NoNewlineAtEof => {
                out.extend_from_slice(b"\\ No newline at end of file\n");
                continue;
            }
        }
        out.extend_from_slice(&line.text);
        out.push(b'\n');
    }
    out
}

fn expected(dir: &Path, flags: &[&str], paths: &[&str]) -> Vec<u8> {
    let mut args = vec![
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--unified=3",
    ];
    args.extend_from_slice(flags);
    args.push("--");
    args.extend_from_slice(paths);
    let out = git_output(dir, &args).stdout;
    let mut body = Vec::new();
    let mut in_body = false;
    for line in out.split_inclusive(|&b| b == b'\n') {
        if line.starts_with(b"@@ ") {
            in_body = true;
        }
        if in_body {
            body.extend_from_slice(line);
        }
    }
    body
}

fn numbered(count: usize) -> String {
    (1..=count).map(|i| format!("line {i}\n")).collect()
}

fn executable(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn ranges_match_git_for_unstaged_staged_and_untracked_files() {
    let repo = init_repo();
    let dir_buf = repo.path().to_path_buf();
    let dir = dir_buf.as_path();
    write(dir, "m.txt", &numbered(60));
    write(dir, "s.txt", &numbered(10));
    commit_all(dir, "base");
    write(
        dir,
        "m.txt",
        &numbered(60)
            .replace("line 5\n", "five\n")
            .replace("line 55\n", "fifty-five\nextra\n"),
    );
    write(dir, "s.txt", &numbered(10).replace("line 3\n", "three\n"));
    git(dir, &["add", "s.txt"]);
    write(dir, "u.txt", "first\nlast without newline");
    let fixture = Fixture::new(repo).await;

    let m = fixture.doc(DiffSide::Unstaged, "m.txt");
    let lines = all_lines(&m).await;
    assert_eq!(render(&lines), expected(dir, &[], &["m.txt"]));
    let meta = m.meta();
    assert_eq!(meta.kind, Some(DocumentKind::Text));
    assert_eq!(meta.hunks.len(), 2);
    assert_eq!(meta.total_lines(), Some(lines.len() as u64));
    assert_eq!(
        lines[meta.hunks[1].first_line as usize].kind,
        DiffLineKind::HunkHeader
    );

    let s = fixture.doc(DiffSide::Staged, "s.txt");
    assert_eq!(
        render(&all_lines(&s).await),
        expected(dir, &["--cached"], &["s.txt"])
    );

    let u = fixture.doc(DiffSide::Unstaged, "u.txt");
    let untracked = all_lines(&u).await;
    assert_eq!(untracked.last().unwrap().kind, DiffLineKind::NoNewlineAtEof);
    assert_eq!(
        render(&untracked),
        expected(dir, &["--no-index"], &["/dev/null", "u.txt"])
            .into_iter()
            .collect::<Vec<u8>>()
    );
    assert!(
        untracked
            .iter()
            .filter(|l| l.kind == DiffLineKind::Added)
            .all(|l| l.old_number.is_none())
    );
}

#[tokio::test]
async fn a_repository_without_a_first_commit_diffs_against_an_empty_base() {
    let repo = init_repo();
    let dir_buf = repo.path().to_path_buf();
    let dir = dir_buf.as_path();
    write(dir, "a.txt", "a\nb\n");
    git(dir, &["add", "a.txt"]);
    write(dir, "b.txt", "x\n");
    let fixture = Fixture::new(repo).await;
    assert!(!fixture.info.has_commits);

    let staged = all_lines(&fixture.doc(DiffSide::Staged, "a.txt")).await;
    assert_eq!(render(&staged), expected(dir, &["--cached"], &["a.txt"]));
    assert_eq!(
        staged
            .iter()
            .filter(|l| l.kind == DiffLineKind::Added)
            .count(),
        2
    );

    let untracked = all_lines(&fixture.doc(DiffSide::Unstaged, "b.txt")).await;
    assert_eq!(render(&untracked), b"@@ -0,0 +1 @@\n+x\n");
}

#[tokio::test]
async fn staged_renames_keep_both_paths_and_pure_renames_are_explicit() {
    let repo = init_repo();
    let dir_buf = repo.path().to_path_buf();
    let dir = dir_buf.as_path();
    write(dir, "old.txt", &numbered(20));
    write(dir, "same.txt", &numbered(20).replace("line", "other"));
    commit_all(dir, "base");
    git(dir, &["mv", "old.txt", "new.txt"]);
    write(dir, "new.txt", &numbered(20).replace("line 10\n", "ten\n"));
    git(dir, &["add", "new.txt"]);
    git(dir, &["mv", "same.txt", "moved.txt"]);
    let fixture = Fixture::new(repo).await;

    let edited = fixture.doc(DiffSide::Staged, "new.txt");
    assert_eq!(
        render(&all_lines(&edited).await),
        expected(dir, &["--cached", "-M"], &["old.txt", "new.txt"])
    );
    assert_eq!(
        edited.key().file.original_path.as_deref(),
        Some(Path::new("old.txt"))
    );

    let pure = fixture.doc(DiffSide::Staged, "moved.txt");
    let meta = pure.wait_finished(&Cancel::new()).await.unwrap();
    assert_eq!(
        meta.kind,
        Some(DocumentKind::RenameOnly {
            from: PathBuf::from("same.txt"),
            to: PathBuf::from("moved.txt")
        })
    );
    assert_eq!(
        pure.read_range(0..10, &Cancel::new()).await.unwrap_err(),
        RangeError::NotReadable(meta.kind.unwrap())
    );
}

#[tokio::test]
async fn special_cases_are_explicit_documents_never_an_empty_diff() {
    let repo = init_repo();
    let dir_buf = repo.path().to_path_buf();
    let dir = dir_buf.as_path();
    write(dir, "text.txt", "a\nb\n");
    write(dir, "mode.sh", "echo\n");
    write(dir, "kind.txt", "a\n");
    write(dir, "locked.txt", "a\n");
    write(dir, "reverted.txt", "a\n");
    std::fs::write(dir.join("bin.dat"), b"\0\x01old").unwrap();
    commit_all(dir, "base");
    std::fs::write(dir.join("bin.dat"), b"\0\x02new").unwrap();
    std::fs::set_permissions(dir.join("mode.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::remove_file(dir.join("kind.txt")).unwrap();
    std::os::unix::fs::symlink("target", dir.join("kind.txt")).unwrap();
    write(dir, "locked.txt", "changed\n");
    std::fs::set_permissions(
        dir.join("locked.txt"),
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    write(dir, "reverted.txt", "changed\n");
    let fixture = Fixture::new(repo).await;
    write(dir, "reverted.txt", "a\n");

    let cancel = Cancel::new();
    let meta_of = |path: &'static str| {
        let doc = fixture.doc(DiffSide::Unstaged, path);
        async move {
            (
                doc.clone(),
                doc.wait_finished(&Cancel::new()).await.unwrap(),
            )
        }
    };

    let (_, binary) = meta_of("bin.dat").await;
    assert_eq!(binary.kind, Some(DocumentKind::Binary));
    assert_eq!(binary.lines, 0);

    let (_, mode) = meta_of("mode.sh").await;
    assert_eq!(
        mode.kind,
        Some(DocumentKind::ModeChange {
            old: "100644".into(),
            new: "100755".into()
        })
    );

    let (kind_doc, kind) = meta_of("kind.txt").await;
    assert_eq!(kind.kind, Some(DocumentKind::Text));
    assert!(kind.type_changed);
    assert!(
        all_lines(&kind_doc)
            .await
            .iter()
            .any(|l| l.kind == DiffLineKind::Removed)
    );

    let (locked_doc, locked) = meta_of("locked.txt").await;
    let Some(DocumentKind::Unreadable(diagnostic)) = locked.kind.clone() else {
        panic!("expected Unreadable, got {:?}", locked.kind)
    };
    assert_eq!(diagnostic.exit_code, Some(128));
    assert!(diagnostic.message.contains("Permission denied"));
    assert!(matches!(
        locked_doc.read_range(0..5, &cancel).await,
        Err(RangeError::NotReadable(_))
    ));
    std::fs::set_permissions(
        dir.join("locked.txt"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    let (reverted_doc, reverted) = meta_of("reverted.txt").await;
    assert_eq!(reverted.kind, Some(DocumentKind::NoChanges));
    assert!(matches!(
        reverted_doc.read_range(0..5, &cancel).await,
        Err(RangeError::NotReadable(DocumentKind::NoChanges))
    ));
}

#[tokio::test]
async fn a_conflicted_file_is_a_conflict_document_in_both_views() {
    let repo = init_repo();
    let dir_buf = repo.path().to_path_buf();
    let dir = dir_buf.as_path();
    write(dir, "c.txt", "base\n");
    commit_all(dir, "base");
    git(dir, &["checkout", "-q", "-b", "other"]);
    write(dir, "c.txt", "other\n");
    commit_all(dir, "other");
    git(dir, &["checkout", "-q", "main"]);
    write(dir, "c.txt", "main\n");
    commit_all(dir, "main");
    assert!(!git_output(dir, &["merge", "other"]).status.success());
    let fixture = Fixture::new(repo).await;

    for side in [DiffSide::Unstaged, DiffSide::Staged] {
        let doc = fixture.doc(side, "c.txt");
        assert_eq!(
            doc.wait_finished(&Cancel::new()).await.unwrap().kind,
            Some(DocumentKind::Conflict)
        );
    }
}

#[tokio::test]
async fn a_file_outside_the_requested_view_is_refused() {
    let repo = init_repo();
    write(repo.path(), "a.txt", "a\n");
    commit_all(repo.path(), "base");
    write(repo.path(), "a.txt", "b\n");
    let fixture = Fixture::new(repo).await;
    let entry = fixture.entry("a.txt");
    let result =
        fixture
            .reader
            .open_document(&fixture.info, fixture.generation, DiffSide::Staged, &entry);
    assert_eq!(result.err(), Some(DiffError::NotInThisView));
}

#[tokio::test]
async fn paths_with_unusual_characters_are_matched_literally() {
    let repo = init_repo();
    let dir_buf = repo.path().to_path_buf();
    let dir = dir_buf.as_path();
    let names: Vec<OsString> = vec![
        "glob[1]*.txt".into(),
        "glob1x.txt".into(),
        "new\nline.txt".into(),
        "-dash.txt".into(),
        ":colon.txt".into(),
        OsString::from_vec(b"bad\xffname.txt".to_vec()),
    ];
    for name in &names {
        std::fs::write(dir.join(name), "old\n").unwrap();
    }
    commit_all(dir, "base");
    std::fs::write(dir.join("glob[1]*.txt"), "changed\n").unwrap();
    for name in &names[2..] {
        std::fs::write(dir.join(name), "changed\n").unwrap();
    }
    let fixture = Fixture::new(repo).await;

    for name in [&names[0], &names[2], &names[3], &names[4], &names[5]] {
        let doc = fixture.doc(DiffSide::Unstaged, name);
        let lines = all_lines(&doc).await;
        assert_eq!(render(&lines), b"@@ -1 +1 @@\n-old\n+changed\n", "{name:?}");
    }
    assert!(
        fixture
            .status
            .entries
            .iter()
            .all(|e| e.path != Path::new("glob1x.txt"))
    );
}

#[tokio::test]
async fn a_fifty_thousand_line_file_is_read_in_ranges_under_small_budgets() {
    let repo = init_repo();
    let dir_buf = repo.path().to_path_buf();
    let dir = dir_buf.as_path();
    let content: String = (0..50_000)
        .map(|i| format!("line {i:05} with some padding text to give it weight\n"))
        .collect();
    write(dir, "big.txt", &content);
    let (memory, disk) = (128 * 1024, 512 * 1024);
    let fixture = Fixture::with_budgets(repo, Some((memory, disk))).await;
    let doc = fixture.doc(DiffSide::Unstaged, "big.txt");
    let cancel = Cancel::new();

    let first = doc.read_range(0..20, &cancel).await.unwrap();
    assert_eq!(first.lines[0].kind, DiffLineKind::HunkHeader);
    assert_eq!(
        first.lines[1].text,
        b"line 00000 with some padding text to give it weight"
    );

    let meta = doc.wait_finished(&cancel).await.unwrap();
    assert_eq!(meta.lines, 50_001);
    assert_eq!(meta.hunks.len(), 1);
    let stats = fixture.reader.cache().stats();
    assert!(
        stats.memory_bytes <= memory && stats.disk_bytes <= disk,
        "{stats:?}"
    );
    assert!(stats.disk_bytes > 0, "nothing was spilled to disk");

    let lines: Vec<&str> = content.lines().collect();
    for start in [1u64, 1023, 1024, 2047, 25_000, 49_990] {
        let data = doc.read_range(start..start + 30, &cancel).await.unwrap();
        for (offset, line) in data.lines.iter().enumerate() {
            let row = start as usize + offset - 1;
            assert_eq!(
                line.text,
                lines[row].as_bytes(),
                "line {}",
                start as usize + offset
            );
            assert_eq!(line.new_number, Some(row as u32 + 1));
        }
        let stats = fixture.reader.cache().stats();
        assert!(
            stats.memory_bytes <= memory && stats.disk_bytes <= disk,
            "{stats:?}"
        );
    }
    let everything = all_lines(&doc).await;
    assert_eq!(everything.len(), 50_001);
    assert!(matches!(
        doc.read_range(60_000..60_010, &cancel).await.unwrap_err(),
        RangeError::OutOfRange {
            requested: 60_000,
            total: 50_001
        }
    ));

    let root = fixture.reader.cache().root().to_path_buf();
    assert!(
        !root.starts_with(dir)
            && root.starts_with(fixture.cache_dir.path().canonicalize().unwrap())
    );
    assert!(std::fs::read_dir(&root).unwrap().count() > 0);
}

#[tokio::test]
async fn cache_entries_are_keyed_by_generation_side_and_file_and_retire_with_their_generation() {
    let repo = init_repo();
    let dir_buf = repo.path().to_path_buf();
    let dir = dir_buf.as_path();
    write(dir, "f.txt", &numbered(10));
    commit_all(dir, "base");
    write(dir, "f.txt", &numbered(10).replace("line 2\n", "staged\n"));
    git(dir, &["add", "f.txt"]);
    write(
        dir,
        "f.txt",
        &numbered(10)
            .replace("line 2\n", "staged\n")
            .replace("line 8\n", "worktree\n"),
    );
    let mut fixture = Fixture::new(repo).await;

    let first = fixture.doc(DiffSide::Unstaged, "f.txt");
    let again = fixture.doc(DiffSide::Unstaged, "f.txt");
    let staged = fixture.doc(DiffSide::Staged, "f.txt");
    assert_eq!(first.id(), again.id());
    assert_ne!(first.id(), staged.id());
    let before = render(&all_lines(&first).await);
    assert!(String::from_utf8_lossy(&before).contains("+worktree"));
    assert!(String::from_utf8_lossy(&render(&all_lines(&staged).await)).contains("+staged"));

    write(
        dir,
        "f.txt",
        &numbered(10)
            .replace("line 2\n", "staged\n")
            .replace("line 8\n", "rewritten\n"),
    );
    let old_generation = fixture.generation;
    fixture.refresh().await;
    let second = fixture.doc(DiffSide::Unstaged, "f.txt");
    assert_ne!(first.id(), second.id());
    assert!(String::from_utf8_lossy(&render(&all_lines(&second).await)).contains("+rewritten"));
    assert_eq!(
        render(&all_lines(&first).await),
        before,
        "an older generation keeps its own content"
    );

    let segments_before = fixture.reader.cache().stats().segments;
    fixture
        .reader
        .retire_before(fixture.info.id, fixture.generation);
    assert!(first.is_retired() && staged.is_retired() && !second.is_retired());
    assert_eq!(
        first.read_range(0..5, &Cancel::new()).await.unwrap_err(),
        RangeError::Stale
    );
    assert!(fixture.reader.cache().stats().segments < segments_before);
    assert_eq!(first.key().generation, old_generation);
    assert!(!all_lines(&second).await.is_empty());

    fixture.reader.close_session(fixture.info.id);
    assert_eq!(
        second.read_range(0..5, &Cancel::new()).await.unwrap_err(),
        RangeError::Stale
    );
    assert_eq!(fixture.reader.cache().stats().segments, 0);
}

#[tokio::test]
async fn an_evicted_segment_is_never_rebuilt_from_a_changed_file() {
    let repo = init_repo();
    let dir_buf = repo.path().to_path_buf();
    let dir = dir_buf.as_path();
    let content: String = (0..6000)
        .map(|i| format!("row {i:05} padding padding padding padding\n"))
        .collect();
    write(dir, "big.txt", &content);
    let fixture = Fixture::with_budgets(repo, Some((2048, 2048))).await;
    let doc = fixture.doc(DiffSide::Unstaged, "big.txt");
    let cancel = Cancel::new();
    doc.wait_finished(&cancel).await.unwrap();

    let untouched = doc.read_range(1..10, &cancel).await.unwrap();
    assert_eq!(
        untouched.lines[0].text,
        b"row 00000 padding padding padding padding"
    );

    write(dir, "big.txt", &format!("a new first line\n{content}"));
    assert_eq!(
        doc.read_range(1..10, &cancel).await.unwrap_err(),
        RangeError::Stale
    );
}

#[tokio::test]
async fn cancelling_a_document_stops_and_reaps_git_and_marks_it_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let fake = dir.path().join("fake-git");
    executable(
        &fake,
        &format!(
            "case \"$*\" in *\" config \"*) exit 1;; esac\necho $$ > '{}'\nprintf '@@ -0,0 +1,2 @@\\n+a\\n+b\\n'\nexec sleep 60",
            pid_file.display()
        ),
    );
    let cache_dir = tempfile::tempdir().unwrap();
    let reader = DiffReader::new(
        Git::with_program(fake),
        DiffCache::open(CacheConfig::new(cache_dir.path())).unwrap(),
    );
    let info = SessionInfo {
        id: gitpoulpe_core::SessionId::next(),
        repo: gitpoulpe_core::RepoId::from_common_dir(&dir.path().join(".git")),
        kind: gitpoulpe_core::RepoKind::Ordinary,
        worktree: Some(dir.path().to_path_buf()),
        git_dir: dir.path().join(".git"),
        common_dir: dir.path().join(".git"),
        has_commits: true,
    };
    let entry = ChangeEntry {
        path: PathBuf::from("f.txt"),
        original_path: None,
        kind: EntryKind::Ordinary,
        index: StatusCode::Unmodified,
        worktree: StatusCode::Modified,
    };
    let doc = reader
        .open_document(
            &info,
            Generation::default().next(),
            DiffSide::Unstaged,
            &entry,
        )
        .unwrap();
    let cancel = Cancel::new();

    let partial = doc.read_range(0..3, &cancel).await.unwrap();
    assert_eq!(partial.lines.len(), 3);
    assert_eq!(
        partial.total_lines, None,
        "the document is still being produced"
    );
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(Path::new(&format!("/proc/{pid}")).exists());

    doc.cancel();
    let meta = tokio::time::timeout(Duration::from_secs(10), doc.wait_finished(&cancel))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.state, BuildState::Cancelled);
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "git was left running or unreaped"
    );
    assert_eq!(
        doc.read_range(0..3, &cancel).await.unwrap_err(),
        RangeError::Cancelled
    );
}

#[tokio::test]
async fn waiting_for_a_range_can_be_cancelled_without_stopping_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("fake-git");
    executable(
        &fake,
        "case \"$*\" in *\" config \"*) exit 1;; esac\nprintf '@@ -0,0 +1,1 @@\\n+a\\n'\nexec sleep 60",
    );
    let cache_dir = tempfile::tempdir().unwrap();
    let reader = DiffReader::new(
        Git::with_program(fake),
        DiffCache::open(CacheConfig::new(cache_dir.path())).unwrap(),
    );
    let info = SessionInfo {
        id: gitpoulpe_core::SessionId::next(),
        repo: gitpoulpe_core::RepoId::from_common_dir(&dir.path().join(".git")),
        kind: gitpoulpe_core::RepoKind::Ordinary,
        worktree: Some(dir.path().to_path_buf()),
        git_dir: dir.path().join(".git"),
        common_dir: dir.path().join(".git"),
        has_commits: true,
    };
    let entry = ChangeEntry {
        path: PathBuf::from("f.txt"),
        original_path: None,
        kind: EntryKind::Ordinary,
        index: StatusCode::Unmodified,
        worktree: StatusCode::Modified,
    };
    let doc = reader
        .open_document(
            &info,
            Generation::default().next(),
            DiffSide::Unstaged,
            &entry,
        )
        .unwrap();
    let waiting = Cancel::new();
    let canceller = waiting.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        canceller.cancel();
    });
    assert_eq!(
        doc.read_range(0..100, &waiting).await.unwrap_err(),
        RangeError::Cancelled
    );
    assert_eq!(doc.meta().state, BuildState::Building);
    assert_eq!(
        doc.read_range(0..2, &Cancel::new())
            .await
            .unwrap()
            .lines
            .len(),
        2
    );
    doc.cancel();
    doc.wait_finished(&Cancel::new()).await.unwrap();
}

#[tokio::test]
async fn repository_configured_programs_are_never_run_to_build_a_diff() {
    let repo = init_repo();
    let dir_buf = repo.path().to_path_buf();
    let dir = dir_buf.as_path();
    write(
        dir,
        ".gitattributes",
        "*.txt filter=evil\n*.bin diff=evil\n",
    );
    write(dir, "f.txt", "one\n");
    std::fs::write(dir.join("t.bin"), "binary-ish one\n").unwrap();
    commit_all(dir, "base");
    let tools = tempfile::tempdir().unwrap();
    let marker = tools.path().join("ran");
    let script = tools.path().join("tool.sh");
    executable(
        &script,
        &format!("touch '{}'\ncat \"$@\"", marker.display()),
    );
    let script = script.to_str().unwrap();
    for (key, value) in [
        ("filter.evil.clean", script),
        ("filter.evil.process", script),
        ("diff.external", script),
        ("diff.evil.textconv", script),
        ("diff.evil.command", script),
        ("core.fsmonitor", script),
    ] {
        git(dir, &["config", key, value]);
    }
    write(dir, "f.txt", "two\n");
    std::fs::write(dir.join("t.bin"), "binary-ish two\n").unwrap();
    let fixture = Fixture::new(repo).await;

    for path in ["f.txt", "t.bin"] {
        let doc = fixture.doc(DiffSide::Unstaged, path);
        let lines = all_lines(&doc).await;
        assert!(
            lines.iter().any(|l| l.kind == DiffLineKind::Added),
            "{path}"
        );
    }
    assert!(
        !marker.exists(),
        "a repository-configured program was executed"
    );
}

#[tokio::test]
async fn reading_diffs_never_changes_the_repository() {
    let repo = init_repo();
    let dir_buf = repo.path().to_path_buf();
    let dir = dir_buf.as_path();
    write(dir, "a.txt", &numbered(30));
    write(dir, "locked.txt", "a\n");
    commit_all(dir, "base");
    write(dir, "a.txt", &numbered(30).replace("line 9\n", "nine\n"));
    git(dir, &["add", "a.txt"]);
    write(
        dir,
        "a.txt",
        &numbered(30)
            .replace("line 9\n", "nine\n")
            .replace("line 20\n", "twenty\n"),
    );
    write(dir, "u.txt", "u\n");
    write(dir, "locked.txt", "b\n");
    std::fs::set_permissions(
        dir.join("locked.txt"),
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    let before = tree_fingerprint_skipping(dir, "locked.txt");

    let fixture = Fixture::new(repo).await;
    for (side, path) in [
        (DiffSide::Unstaged, "a.txt"),
        (DiffSide::Staged, "a.txt"),
        (DiffSide::Unstaged, "u.txt"),
        (DiffSide::Unstaged, "locked.txt"),
    ] {
        let doc = fixture.doc(side, path);
        let _ = doc.wait_finished(&Cancel::new()).await.unwrap();
        let _ = doc.read_range(0..50, &Cancel::new()).await;
    }
    let cancelled = fixture.doc(DiffSide::Unstaged, "a.txt");
    cancelled.cancel();
    let _ = cancelled.wait_finished(&Cancel::new()).await;

    assert_eq!(tree_fingerprint_skipping(dir, "locked.txt"), before);
    std::fs::set_permissions(
        dir.join("locked.txt"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
}

fn tree_fingerprint_skipping(
    root: &Path,
    skip: &str,
) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut map = tree_fingerprint_unreadable_safe(root);
    map.remove(Path::new(skip));
    map
}

fn tree_fingerprint_unreadable_safe(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if entry.file_type().unwrap().is_dir() {
                out.insert(relative, b"<dir>".to_vec());
                stack.push(path);
            } else {
                out.insert(relative, std::fs::read(&path).unwrap_or_default());
            }
        }
    }
    out
}

#[tokio::test]
async fn a_cache_directory_inside_the_repository_is_refused() {
    let repo = init_repo();
    write(repo.path(), "a.txt", "a\n");
    let inside = repo.path().join(".cache-inside");
    let git = Git::new();
    let reader = DiffReader::new(
        git.clone(),
        DiffCache::open(CacheConfig::new(&inside)).unwrap(),
    );
    let cancel = Cancel::new();
    let info = git.open(repo.path(), &cancel).await.unwrap();
    let status = git
        .read_status(&info, Generation::default().next(), &cancel)
        .await
        .unwrap();
    let entry = status
        .entries
        .iter()
        .find(|e| e.path == Path::new("a.txt"))
        .unwrap();
    let result = reader.open_document(
        &info,
        Generation::default().next(),
        DiffSide::Unstaged,
        entry,
    );
    assert_eq!(result.err(), Some(DiffError::CacheInsideRepository));
}

#[tokio::test]
async fn a_partial_clone_never_fetches_missing_objects_over_the_network() {
    let source = init_repo();
    write(source.path(), "f.txt", "hello\n");
    commit_all(source.path(), "base");
    git(source.path(), &["config", "uploadpack.allowFilter", "true"]);
    git(
        source.path(),
        &["config", "uploadpack.allowAnySHA1InWant", "true"],
    );
    let holder = tempfile::tempdir().unwrap();
    let url = format!("file://{}", source.path().display());
    git(
        holder.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "clone",
            "-q",
            "--filter=blob:none",
            "--no-checkout",
            &url,
            "dst",
        ],
    );
    let dst = holder.path().join("dst");
    let tools = tempfile::tempdir().unwrap();
    let marker = tools.path().join("ran");
    let script = tools.path().join("ssh.sh");
    executable(&script, &format!("touch '{}'\nexit 1", marker.display()));
    git(
        &dst,
        &["config", "core.sshCommand", script.to_str().unwrap()],
    );
    git(
        &dst,
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://example.invalid/repo.git",
        ],
    );

    let fixture = Fixture::build(holder, dst, None).await;
    let doc = fixture.doc(DiffSide::Staged, "f.txt");
    let meta = doc.wait_finished(&Cancel::new()).await.unwrap();

    assert!(
        !marker.exists(),
        "a repository-configured network command was executed"
    );
    assert!(
        matches!(meta.kind, Some(DocumentKind::Unreadable(_))),
        "{:?}",
        meta.kind
    );
}
