mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{git as run_git, init_repo, write};
use gitpoulpe_core::{LfsFileState, LfsTransfer, SessionInfo};
use gitpoulpe_git::{Cancel, Git, LfsError, NetError, parse_pointer};
use sha2::{Digest, Sha256};

type Check = fn(&LfsError) -> bool;

fn oid_of(content: &[u8]) -> String {
    Sha256::digest(content)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn pointer_text(oid: &str, size: usize) -> String {
    format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize {size}\n")
}

struct Env {
    repo: tempfile::TempDir,
    git: Git,
    info: SessionInfo,
}

impl Env {
    fn path(&self) -> &Path {
        self.repo.path()
    }
    fn store(&self, oid: &str, content: &[u8]) {
        let dir = self
            .path()
            .join(".git/lfs/objects")
            .join(&oid[..2])
            .join(&oid[2..4]);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(oid), content).unwrap();
    }
}

async fn env_with_path(prefix: Option<&Path>) -> Env {
    let repo = init_repo();
    run_git(
        repo.path(),
        &["remote", "add", "origin", "https://127.0.0.1:9/none.git"],
    );
    let mut git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com");
    if let Some(dir) = prefix {
        let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap());
        git = git.with_env("PATH", path);
    }
    let info = git.open(repo.path(), &Cancel::new()).await.unwrap();
    Env { repo, git, info }
}

fn lfs_repo(e: &Env, files: &[(&str, &[u8])]) {
    write(
        e.path(),
        ".gitattributes",
        "*.bin filter=lfs diff=lfs merge=lfs -text\n",
    );
    for (name, content) in files {
        write(
            e.path(),
            name,
            &pointer_text(&oid_of(content), content.len()),
        );
    }
    run_git(e.path(), &["add", "-A"]);
    run_git(e.path(), &["commit", "-q", "-m", "with lfs pointers"]);
}

fn fake_lfs(dir: &Path, body: &str) {
    let script = dir.join("git-lfs");
    std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn pointers_are_recognised_strictly() {
    let oid = "a".repeat(64);
    let good = pointer_text(&oid, 12);
    let parsed = parse_pointer(good.as_bytes()).unwrap();
    assert_eq!((parsed.oid.as_str(), parsed.size), (oid.as_str(), 12));
    assert!(parse_pointer(format!("{good}extra key value\n").as_bytes()).is_some());
    for bad in [
        String::new(),
        "just a file\n".to_owned(),
        format!("version https://git-lfs.github.com/spec/v2\noid sha256:{oid}\nsize 1\n"),
        format!(
            "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 1\n",
            "A".repeat(64)
        ),
        format!(
            "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 1\n",
            "a".repeat(63)
        ),
        format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\n"),
        format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize -1\n"),
        format!("{good}{}", "x".repeat(2000)),
    ] {
        assert!(parse_pointer(bad.as_bytes()).is_none(), "{bad:?}");
    }
    assert!(parse_pointer(&[0xff, 0xfe, 0x00]).is_none());
}

#[tokio::test]
async fn a_pointer_is_told_apart_from_real_content_and_a_missing_object_is_reported() {
    let e = env_with_path(None).await;
    let present: &[u8] = b"the real big file content";
    let only_pointer: &[u8] = b"object that is in the store";
    let missing: &[u8] = b"object nobody has";
    let modified: &[u8] = b"original content";
    let same_size: &[u8] = b"AAAAAAAAAAAAAAAA";
    lfs_repo(
        &e,
        &[
            ("present.bin", present),
            ("pointer-only.bin", only_pointer),
            ("missing.bin", missing),
            ("modified.bin", modified),
            ("same-size.bin", same_size),
        ],
    );
    std::fs::write(e.path().join("same-size.bin"), b"BBBBBBBBBBBBBBBB").unwrap();
    std::fs::write(e.path().join("present.bin"), present).unwrap();
    e.store(&oid_of(present), present);
    e.store(&oid_of(only_pointer), only_pointer);
    std::fs::write(
        e.path().join("modified.bin"),
        b"edited and no longer the committed object",
    )
    .unwrap();
    write(e.path(), "plain.txt", "not lfs\n");
    write(
        e.path(),
        "converted.bin",
        "committed as real content before LFS\n",
    );
    run_git(e.path(), &["add", "-f", "converted.bin"]);
    let files = e.git.list_lfs_files(&e.info).await.unwrap();
    let state = |n: &str| files.iter().find(|f| f.path == Path::new(n)).unwrap().state;
    assert_eq!(state("present.bin"), LfsFileState::Present);
    assert_eq!(state("pointer-only.bin"), LfsFileState::PointerOnly);
    assert_eq!(state("missing.bin"), LfsFileState::Missing);
    assert_eq!(state("modified.bin"), LfsFileState::Modified);
    assert_eq!(state("same-size.bin"), LfsFileState::Modified);
    assert_eq!(state("converted.bin"), LfsFileState::NotConverted);
    assert!(files.iter().all(|f| f.path != Path::new("plain.txt")));
    let missing_file = files
        .iter()
        .find(|f| f.path == Path::new("missing.bin"))
        .unwrap();
    assert_eq!(missing_file.pointer.as_ref().unwrap().oid, oid_of(missing));
    assert_eq!(
        missing_file.pointer.as_ref().unwrap().size,
        missing.len() as u64
    );
}

#[tokio::test]
async fn a_deleted_worktree_file_follows_the_object_store() {
    let e = env_with_path(None).await;
    let content: &[u8] = b"data";
    lfs_repo(&e, &[("a.bin", content)]);
    std::fs::remove_file(e.path().join("a.bin")).unwrap();
    assert_eq!(
        e.git.list_lfs_files(&e.info).await.unwrap()[0].state,
        LfsFileState::Missing
    );
    e.store(&oid_of(content), content);
    assert_eq!(
        e.git.list_lfs_files(&e.info).await.unwrap()[0].state,
        LfsFileState::PointerOnly
    );
}

#[tokio::test]
async fn without_git_lfs_the_support_is_explained_and_nothing_is_installed_or_downloaded() {
    let only_git = tempfile::tempdir().unwrap();
    let real_git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("git"))
        .find(|candidate| candidate.is_file())
        .expect("git is on the PATH");
    std::os::unix::fs::symlink(real_git, only_git.path().join("git")).unwrap();
    let e = env_with_path(None).await;
    let git = e.git.clone().with_env("PATH", only_git.path().as_os_str());
    lfs_repo(&e, &[("a.bin", b"x")]);
    let status = git.lfs_status(&e.info).await.unwrap();
    assert!(!status.installed && status.version.is_none());
    assert!(status.uses_lfs, "the repository still declares LFS");
    assert!(status.detail.contains("git lfs install"));
    assert!(status.detail.contains("n'installe ni ne télécharge rien"));
    let error = git
        .lfs_transfer(
            &e.info,
            LfsTransfer::Pull,
            "origin",
            "main",
            &Cancel::new(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&error, LfsError::NotInstalled(h) if h.contains("gestionnaire de paquets")),
        "{error:?}"
    );
    assert!(!e.path().join(".git/lfs").exists(), "nothing was created");
    assert!(
        e.git.list_lfs_files(&e.info).await.unwrap().len() == 1,
        "reading pointers needs no tool"
    );
}

#[tokio::test]
async fn a_transfer_shows_its_progress_and_ends_cleanly() {
    let tools = tempfile::tempdir().unwrap();
    fake_lfs(
        tools.path(),
        r#"case "$1" in
  version) echo "git-lfs/3.5.1 (test double)" ;;
  fetch|pull|push)
    echo "Downloading LFS objects:  33% (1/3), 1.2 MB | 500 KB/s" >&2
    echo "Downloading LFS objects:  66% (2/3), 2.4 MB | 500 KB/s" >&2
    echo "Downloading LFS objects: 100% (3/3), 3.6 MB | 500 KB/s, done." >&2 ;;
esac"#,
    );
    let e = env_with_path(Some(tools.path())).await;
    let status = e.git.lfs_status(&e.info).await.unwrap();
    assert!(status.installed);
    assert_eq!(
        status.version.as_deref(),
        Some("git-lfs/3.5.1 (test double)")
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    e.git
        .lfs_transfer(
            &e.info,
            LfsTransfer::Fetch,
            "origin",
            "main",
            &Cancel::new(),
            move |p| {
                sink.lock().unwrap().push((p.phase, p.percent));
            },
        )
        .await
        .unwrap();
    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        seen.iter().map(|(_, p)| *p).collect::<Vec<_>>(),
        [33, 66, 100]
    );
    assert!(
        seen.iter()
            .all(|(phase, _)| phase == "Downloading LFS objects")
    );
}

#[tokio::test]
async fn a_cancelled_transfer_stops_the_tool_and_what_it_started() {
    let tools = tempfile::tempdir().unwrap();
    fake_lfs(
        tools.path(),
        &format!(
            r#"case "$1" in
  version) echo "git-lfs/3.5.1" ;;
  *) sleep 60 & echo $! > "{}/sleeper"; wait ;;
esac"#,
            tools.path().display()
        ),
    );
    let e = env_with_path(Some(tools.path())).await;
    let cancel = Cancel::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(700)).await;
        canceller.cancel();
    });
    let error = e
        .git
        .lfs_transfer(
            &e.info,
            LfsTransfer::Push,
            "origin",
            "main",
            &cancel,
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(error, LfsError::Net(NetError::Cancelled));
    let pid = std::fs::read_to_string(tools.path().join("sleeper"))
        .unwrap()
        .trim()
        .to_owned();
    let mut alive = true;
    for _ in 0..60 {
        alive = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|s| !s.contains(") Z"))
            .unwrap_or(false);
        if !alive {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!alive, "the tool's own child survived");
}

#[tokio::test]
async fn server_and_access_errors_are_distinguished_with_filtered_output() {
    let cases: [(&str, Check); 4] = [
        (
            "batch response: Authorization error: token=s3cretvalue",
            |e| matches!(e, LfsError::AccessDenied(m) if !m.contains("s3cretvalue")),
        ),
        ("batch response: Repository or object not found", |e| {
            matches!(e, LfsError::NotFound(_))
        }),
        (
            "Post \"https://host/objects/batch\": 502 Bad Gateway",
            |e| matches!(e, LfsError::Server(_)),
        ),
        (
            "dial tcp: lookup nohost: no such host; could not resolve host",
            |e| matches!(e, LfsError::Net(NetError::Network(_))),
        ),
    ];
    for (stderr, check) in cases {
        let tools = tempfile::tempdir().unwrap();
        fake_lfs(
            tools.path(),
            &format!(
                "case \"$1\" in\n  version) echo git-lfs/3.5.1 ;;\n  *) echo '{stderr}' >&2; exit 2 ;;\nesac"
            ),
        );
        let e = env_with_path(Some(tools.path())).await;
        let error = e
            .git
            .lfs_transfer(
                &e.info,
                LfsTransfer::Pull,
                "origin",
                "main",
                &Cancel::new(),
                |_| {},
            )
            .await
            .unwrap_err();
        assert!(check(&error), "{stderr} -> {error:?}");
    }
}

#[tokio::test]
async fn hostile_remote_and_branch_names_never_reach_the_tool() {
    let tools = tempfile::tempdir().unwrap();
    let log = tools.path().join("calls");
    fake_lfs(
        tools.path(),
        &format!(
            "echo \"$@\" >> '{}'\ncase \"$1\" in version) echo git-lfs/3.5.1 ;; esac",
            log.display()
        ),
    );
    let e = env_with_path(Some(tools.path())).await;
    for (remote, branch) in [
        ("--all", "main"),
        ("origin", "--delete"),
        ("ghost", "main"),
        ("origin", ""),
    ] {
        let error = e
            .git
            .lfs_transfer(
                &e.info,
                LfsTransfer::Push,
                remote,
                branch,
                &Cancel::new(),
                |_| {},
            )
            .await
            .unwrap_err();
        assert_eq!(error, LfsError::InvalidRemote, "{remote} {branch}");
    }
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(!calls.contains("push"), "{calls}");
    let _: PathBuf = PathBuf::new();
}

#[tokio::test]
async fn a_huge_file_or_a_symbolic_link_is_never_read_whole_or_followed() {
    let e = env_with_path(None).await;
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("real");
    let content = "x".repeat(target.display().to_string().len());
    lfs_repo(&e, &[("huge.bin", b"a"), ("link.bin", content.as_bytes())]);
    let huge = std::fs::File::create(e.path().join("huge.bin")).unwrap();
    huge.set_len(6 * 1024 * 1024 * 1024).unwrap();
    std::fs::write(&target, &content).unwrap();
    std::fs::remove_file(e.path().join("link.bin")).unwrap();
    std::os::unix::fs::symlink(&target, e.path().join("link.bin")).unwrap();
    let link_len = std::fs::symlink_metadata(e.path().join("link.bin"))
        .unwrap()
        .len();
    assert_eq!(
        link_len,
        content.len() as u64,
        "same size, same hashed content through the link"
    );
    let started = std::time::Instant::now();
    let files = e.git.list_lfs_files(&e.info).await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let state = |n: &str| files.iter().find(|f| f.path == Path::new(n)).unwrap().state;
    assert_eq!(state("huge.bin"), LfsFileState::Modified);
    assert_eq!(
        state("link.bin"),
        LfsFileState::Modified,
        "a link to matching content is not the object"
    );
}
