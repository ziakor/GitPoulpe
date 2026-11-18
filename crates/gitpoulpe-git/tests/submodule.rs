mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::write;
use gitpoulpe_core::{SessionInfo, SubmoduleState};
use gitpoulpe_git::{Cancel, Git, NetError, SubmoduleError};

fn sg(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_ALLOW_PROTOCOL", "file")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn commit(dir: &Path, file: &str, content: &str, message: &str) -> String {
    write(dir, file, content);
    sg(dir, &["add", "-A"]);
    sg(dir, &["commit", "-q", "-m", message]);
    sg(dir, &["rev-parse", "HEAD"])
}

fn engine() -> Git {
    Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_ALLOW_PROTOCOL", "file")
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com")
}

struct Env {
    area: tempfile::TempDir,
    git: Git,
    info: SessionInfo,
    sub_origin: PathBuf,
    first: String,
}

async fn env() -> Env {
    let area = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(area.path()).unwrap();
    let sub_origin = root.join("sub-origin");
    std::fs::create_dir(&sub_origin).unwrap();
    sg(&sub_origin, &["init", "-q", "-b", "main"]);
    let first = commit(&sub_origin, "lib.txt", "v1\n", "sub one");
    let origin = root.join("super-origin");
    std::fs::create_dir(&origin).unwrap();
    sg(&origin, &["init", "-q", "-b", "main"]);
    sg(
        &origin,
        &[
            "submodule",
            "add",
            "-q",
            &sub_origin.display().to_string(),
            "libs/sub",
        ],
    );
    sg(&origin, &["commit", "-q", "-m", "add submodule"]);
    let work = root.join("work");
    sg(
        &root,
        &["clone", "-q", &origin.display().to_string(), "work"],
    );
    let git = engine();
    let info = git.open(&work, &Cancel::new()).await.unwrap();
    Env {
        area,
        git,
        info,
        sub_origin,
        first,
    }
}

impl Env {
    fn work(&self) -> PathBuf {
        self.info.worktree.clone().unwrap()
    }
    fn path() -> &'static Path {
        Path::new("libs/sub")
    }
    async fn ready(&self) {
        self.git
            .update_submodule(&self.info, Self::path(), false, &Cancel::new(), |_| {})
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn an_uninitialised_submodule_shows_its_expected_commit_and_its_declared_destination() {
    let e = env().await;
    let list = e.git.list_submodules(&e.info).await.unwrap();
    assert_eq!(list.len(), 1);
    let s = &list[0];
    assert_eq!(s.path, PathBuf::from("libs/sub"));
    assert_eq!(s.state, SubmoduleState::NotInitialized);
    assert_eq!(s.expected, e.first);
    assert_eq!(s.current, None);
    assert_eq!(
        s.url.as_deref(),
        Some(e.sub_origin.display().to_string().as_str())
    );
    assert!(s.url_refused.is_none());
    assert!(!e.work().join("libs/sub/lib.txt").exists());
}

#[tokio::test]
async fn initialising_then_updating_checks_out_the_expected_commit_at_the_declared_path() {
    let e = env().await;
    e.git.init_submodule(&e.info, Env::path()).await.unwrap();
    assert_eq!(
        e.git.list_submodules(&e.info).await.unwrap()[0].state,
        SubmoduleState::Initialized
    );
    let done = e
        .git
        .update_submodule(&e.info, Env::path(), false, &Cancel::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(done.to, e.first);
    assert_eq!(done.from, None);
    let s = &e.git.list_submodules(&e.info).await.unwrap()[0];
    assert_eq!(s.state, SubmoduleState::UpToDate);
    assert_eq!(s.current.as_deref(), Some(e.first.as_str()));
    assert_eq!(
        std::fs::read_to_string(e.work().join("libs/sub/lib.txt")).unwrap(),
        "v1\n"
    );
}

#[tokio::test]
async fn a_submodule_at_another_revision_is_told_apart_from_the_expected_one() {
    let e = env().await;
    e.ready().await;
    let sub = e.work().join("libs/sub");
    let local = commit(&sub, "lib.txt", "v2 local\n", "local only");
    let s = &e.git.list_submodules(&e.info).await.unwrap()[0];
    assert_eq!(s.state, SubmoduleState::Differs);
    assert_eq!(s.expected, e.first);
    assert_eq!(s.current.as_deref(), Some(local.as_str()));
    assert_eq!((s.ahead, s.behind), (1, 0));
    assert!(s.expected_available);
}

#[tokio::test]
async fn local_work_in_a_submodule_is_protected_from_an_update() {
    let e = env().await;
    e.ready().await;
    let sub = e.work().join("libs/sub");
    write(&sub, "lib.txt", "edited, uncommitted\n");
    let error = e
        .git
        .update_submodule(&e.info, Env::path(), true, &Cancel::new(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(error, SubmoduleError::Dirty(vec![PathBuf::from("lib.txt")]));
    assert_eq!(
        std::fs::read_to_string(sub.join("lib.txt")).unwrap(),
        "edited, uncommitted\n"
    );
    sg(&sub, &["checkout", "-q", "--", "lib.txt"]);
    let orphan = commit(
        &sub,
        "lib.txt",
        "v2 orphan\n",
        "unpushed work on a detached head",
    );
    let error = e
        .git
        .update_submodule(&e.info, Env::path(), false, &Cancel::new(), |_| {})
        .await
        .unwrap_err();
    assert!(
        matches!(&error, SubmoduleError::UnsavedCommits(c) if c.len() == 1 && c[0].contains("unpushed work")),
        "{error:?}"
    );
    assert_eq!(sg(&sub, &["rev-parse", "HEAD"]), orphan);
    let done = e
        .git
        .update_submodule(&e.info, Env::path(), true, &Cancel::new(), |_| {})
        .await
        .unwrap();
    let marker = done
        .marker
        .expect("a recovery marker keeps the orphan commits");
    assert_eq!(sg(&sub, &["rev-parse", &marker]), orphan);
    assert_eq!(sg(&sub, &["rev-parse", "HEAD"]), e.first);
}

#[tokio::test]
async fn an_unreachable_remote_is_reported_explicitly_and_leaves_the_submodule_as_it_was() {
    let e = env().await;
    e.ready().await;
    let next = commit(&e.sub_origin, "lib.txt", "v2 upstream\n", "sub two");
    let super_origin = e.area.path().join("super-origin");
    let inner = super_origin.join("libs/sub");
    sg(&inner, &["fetch", "-q", "origin"]);
    sg(&inner, &["checkout", "-q", &next]);
    sg(&super_origin, &["add", "libs/sub"]);
    sg(&super_origin, &["commit", "-q", "-m", "bump submodule"]);
    sg(&e.work(), &["pull", "-q", "--no-recurse-submodules"]);
    let list = e.git.list_submodules(&e.info).await.unwrap();
    assert_eq!(list[0].expected, next);
    assert_eq!(list[0].state, SubmoduleState::Differs);
    assert!(
        !list[0].expected_available,
        "the expected commit was never fetched here"
    );
    std::fs::remove_dir_all(&e.sub_origin).unwrap();
    let sub = e.work().join("libs/sub");
    write(&sub, "scratch.txt", "untracked work\n");
    let before = sg(&sub, &["rev-parse", "HEAD"]);
    let error = e
        .git
        .update_submodule(&e.info, Env::path(), false, &Cancel::new(), |_| {})
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            SubmoduleError::Net(NetError::NotFound | NetError::Network(_))
                | SubmoduleError::Refused(_)
        ),
        "{error:?}"
    );
    assert_eq!(sg(&sub, &["rev-parse", "HEAD"]), before);
    assert_eq!(
        std::fs::read_to_string(sub.join("scratch.txt")).unwrap(),
        "untracked work\n"
    );
}

#[tokio::test]
async fn a_nested_submodule_is_listed_with_its_depth_and_updated_through_its_parent() {
    let e = env().await;
    let inner_origin = e.area.path().join("inner-origin");
    std::fs::create_dir(&inner_origin).unwrap();
    sg(&inner_origin, &["init", "-q", "-b", "main"]);
    let inner = commit(&inner_origin, "deep.txt", "deep\n", "inner one");
    let sub = e.sub_origin.clone();
    sg(
        &sub,
        &[
            "submodule",
            "add",
            "-q",
            &inner_origin.display().to_string(),
            "deps/inner",
        ],
    );
    sg(&sub, &["commit", "-q", "-m", "add inner"]);
    e.ready().await;
    sg(&e.work().join("libs/sub"), &["fetch", "-q", "origin"]);
    sg(
        &e.work().join("libs/sub"),
        &["merge", "-q", "--ff-only", "origin/main"],
    );
    sg(&e.work(), &["add", "libs/sub"]);
    sg(&e.work(), &["commit", "-q", "-m", "bump to include inner"]);
    let list = e.git.list_submodules(&e.info).await.unwrap();
    assert_eq!(list.len(), 2, "{list:?}");
    assert_eq!(list[1].path, PathBuf::from("libs/sub/deps/inner"));
    assert_eq!(list[1].depth, 1);
    assert_eq!(list[1].state, SubmoduleState::NotInitialized);
    let done = e
        .git
        .update_submodule(
            &e.info,
            Path::new("libs/sub/deps/inner"),
            false,
            &Cancel::new(),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(done.to, inner);
    assert_eq!(
        std::fs::read_to_string(e.work().join("libs/sub/deps/inner/deep.txt")).unwrap(),
        "deep\n"
    );
}

#[tokio::test]
async fn a_hostile_url_in_gitmodules_is_refused_and_never_executed() {
    let e = env().await;
    let marker = e.area.path().join("pwned");
    let urls = [
        format!("ext::sh -c 'touch {}'", marker.display()),
        "-oProxyCommand=touch pwned".to_owned(),
        "ssh://-oProxyCommand=x/repo".to_owned(),
        "ftp://example.org/x".to_owned(),
    ];
    for url in urls {
        sg(
            &e.work(),
            &[
                "config",
                "-f",
                ".gitmodules",
                "submodule.libs/sub.url",
                &url,
            ],
        );
        let list = e.git.list_submodules(&e.info).await.unwrap();
        assert!(list[0].url_refused.is_some(), "{url}");
        assert!(list[0].url.is_none());
        let error = e
            .git
            .init_submodule(&e.info, Env::path())
            .await
            .unwrap_err();
        assert!(
            matches!(error, SubmoduleError::UrlRefused(_)),
            "{url}: {error:?}"
        );
        let error = e
            .git
            .update_submodule(&e.info, Env::path(), false, &Cancel::new(), |_| {})
            .await
            .unwrap_err();
        assert!(
            matches!(error, SubmoduleError::UrlRefused(_)),
            "{url}: {error:?}"
        );
    }
    assert!(!marker.exists());
    assert!(!e.work().join("libs/sub/lib.txt").exists());
}

#[tokio::test]
async fn unknown_paths_are_refused_and_credentials_in_urls_are_masked() {
    let e = env().await;
    for path in ["nope", "../libs/sub", "libs"] {
        assert_eq!(
            e.git
                .init_submodule(&e.info, Path::new(path))
                .await
                .unwrap_err(),
            SubmoduleError::NoSuchSubmodule,
            "{path}"
        );
    }
    sg(
        &e.work(),
        &[
            "config",
            "-f",
            ".gitmodules",
            "submodule.libs/sub.url",
            "https://user:tok3n@example.org/r.git",
        ],
    );
    let s = &e.git.list_submodules(&e.info).await.unwrap()[0];
    assert_eq!(s.url.as_deref(), Some("https://***@example.org/r.git"));
    let _ = NetError::Cancelled;
}
