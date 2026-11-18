mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{HttpServer, commit_all, git as run_git, init_repo, write};
use gitpoulpe_git::{
    Cancel, CloneError, Git, NetError, repository_name_from_url, valid_directory_name,
};

struct Env {
    dir: tempfile::TempDir,
    source: tempfile::TempDir,
    parent: tempfile::TempDir,
    git: Git,
}

impl Env {
    fn file_url(&self) -> String {
        format!("file://{}", self.source.path().display())
    }
    fn ssh_url(&self) -> String {
        format!("ssh://git@fakehost{}", self.source.path().display())
    }
    fn mode(&self, mode: &str) {
        std::fs::write(self.dir.path().join("mode"), mode).unwrap();
    }
    fn parent(&self) -> &Path {
        self.parent.path()
    }
    fn listing(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.parent())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fakessh");
    std::fs::write(&script, include_str!("common/fake_ssh.sh")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.path().join("mode"), "ok").unwrap();
    let source = init_repo();
    write(source.path(), "a.txt", "one\n");
    commit_all(source.path(), "first");
    run_git(source.path(), &["branch", "dev"]);
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_SSH_COMMAND", script.as_os_str())
        .with_env("FAKESSH_DIR", dir.path().as_os_str());
    Env {
        dir,
        source,
        parent: tempfile::tempdir().unwrap(),
        git,
    }
}

fn head(dir: &Path) -> String {
    String::from_utf8_lossy(&run_git(dir, &["symbolic-ref", "--short", "HEAD"]))
        .trim()
        .to_owned()
}

#[tokio::test]
async fn a_clone_lands_in_the_destination_on_the_remote_default_branch() {
    let e = env();
    let url = e.file_url();
    let path = e
        .git
        .clone_repository(&url, e.parent(), "copy", &Cancel::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(
        path,
        std::fs::canonicalize(e.parent()).unwrap().join("copy")
    );
    assert_eq!(head(&path), "main");
    assert_eq!(
        std::fs::read_to_string(path.join("a.txt")).unwrap(),
        "one\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&run_git(&path, &["remote", "get-url", "origin"])).trim(),
        url
    );
    assert_eq!(e.listing(), ["copy"]);
    let opened = e.git.open(&path, &Cancel::new()).await.unwrap();
    assert!(opened.has_commits);
}

#[tokio::test]
async fn an_existing_empty_folder_is_filled_and_a_non_empty_one_is_refused_untouched() {
    let e = env();
    let url = e.file_url();
    std::fs::create_dir(e.parent().join("empty")).unwrap();
    e.git
        .clone_repository(&url, e.parent(), "empty", &Cancel::new(), |_| {})
        .await
        .unwrap();
    assert!(e.parent().join("empty/a.txt").exists());
    std::fs::create_dir(e.parent().join("full")).unwrap();
    std::fs::write(e.parent().join("full/mine.txt"), "precious").unwrap();
    let error = e
        .git
        .clone_repository(&url, e.parent(), "full", &Cancel::new(), |_| {})
        .await
        .unwrap_err();
    assert!(matches!(error, CloneError::DestinationNotEmpty(_)));
    e.mode("offline");
    let early = e
        .git
        .clone_repository(&e.ssh_url(), e.parent(), "full", &Cancel::new(), |_| {})
        .await
        .unwrap_err();
    assert!(
        matches!(early, CloneError::DestinationNotEmpty(_)),
        "the refusal must come before any network access: {early:?}"
    );
    assert_eq!(
        std::fs::read_to_string(e.parent().join("full/mine.txt")).unwrap(),
        "precious"
    );
    assert_eq!(e.listing(), ["empty", "full"]);
}

#[tokio::test]
async fn bad_input_is_refused_before_anything_is_created() {
    let e = env();
    let url = e.file_url();
    for name in ["", ".", "..", "a/b", "-x", "x\ny"] {
        assert_eq!(
            e.git
                .clone_repository(&url, e.parent(), name, &Cancel::new(), |_| {})
                .await
                .unwrap_err(),
            CloneError::InvalidName,
            "{name:?}"
        );
    }
    assert!(matches!(
        e.git
            .clone_repository("ext::sh -c id", e.parent(), "x", &Cancel::new(), |_| {})
            .await,
        Err(CloneError::InvalidUrl(_))
    ));
    assert_eq!(
        e.git
            .clone_repository(
                "https://user:tok@example.org/r.git",
                e.parent(),
                "x",
                &Cancel::new(),
                |_| {}
            )
            .await
            .unwrap_err(),
        CloneError::SecretInUrl
    );
    assert_eq!(
        e.git
            .clone_repository(
                &url,
                &e.parent().join("missing"),
                "x",
                &Cancel::new(),
                |_| {}
            )
            .await
            .unwrap_err(),
        CloneError::ParentMissing
    );
    assert!(e.listing().is_empty());
}

#[tokio::test]
async fn a_failed_clone_leaves_no_partial_folder_and_a_retry_works() {
    let e = env();
    e.mode("offline");
    let url = e.ssh_url();
    let error = e
        .git
        .clone_repository(&url, e.parent(), "copy", &Cancel::new(), |_| {})
        .await
        .unwrap_err();
    assert!(
        matches!(error, CloneError::Net(NetError::Network(_))),
        "{error:?}"
    );
    assert!(e.listing().is_empty());
    e.mode("ok");
    e.git
        .clone_repository(&url, e.parent(), "copy", &Cancel::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(e.listing(), ["copy"]);
}

#[tokio::test]
async fn cancelling_a_clone_stops_every_process_and_removes_the_partial_folder() {
    let e = env();
    e.mode("hang");
    let url = e.ssh_url();
    let cancel = Cancel::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        canceller.cancel();
    });
    let error = e
        .git
        .clone_repository(&url, e.parent(), "copy", &cancel, |_| {})
        .await
        .unwrap_err();
    assert_eq!(error, CloneError::Net(NetError::Cancelled));
    assert!(e.listing().is_empty(), "{:?}", e.listing());
    let pid = std::fs::read_to_string(e.dir.path().join("sleeper"))
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
    assert!(!alive);
}

#[tokio::test]
async fn a_read_only_parent_is_reported_and_nothing_is_created() {
    let e = env();
    let url = e.file_url();
    std::fs::set_permissions(e.parent(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let writable = std::fs::File::create(e.parent().join("probe")).is_ok();
    if writable {
        std::fs::set_permissions(e.parent(), std::fs::Permissions::from_mode(0o700)).unwrap();
        return;
    }
    let error = e
        .git
        .clone_repository(&url, e.parent(), "copy", &Cancel::new(), |_| {})
        .await
        .unwrap_err();
    std::fs::set_permissions(e.parent(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(error, CloneError::NotWritable);
    assert!(e.listing().is_empty());
}

#[tokio::test]
async fn the_progress_of_a_larger_clone_is_reported() {
    let e = env();
    for i in 0..80 {
        write(e.source.path(), &format!("f{i}.txt"), &format!("{i}\n"));
        commit_all(e.source.path(), &format!("c{i}"));
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let url = e.file_url();
    e.git
        .clone_repository(&url, e.parent(), "copy", &Cancel::new(), move |p| {
            sink.lock().unwrap().push(p)
        })
        .await
        .unwrap();
    assert!(!seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_https_clone_uses_the_users_credential_helper() {
    let e = env();
    let server = HttpServer::start(e.source.path(), Some(("alice", "s3cret")));
    let helper = e.dir.path().join("helper.sh");
    std::fs::write(
        &helper,
        "#!/bin/sh\necho username=alice\necho password=s3cret\n",
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = e.dir.path().join("gitconfig");
    std::fs::write(
        &config,
        format!(
            "[http]\n\tsslCAInfo = {}\n[credential]\n\thelper = {}\n",
            server.cert().display(),
            helper.display()
        ),
    )
    .unwrap();
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", config.as_os_str())
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null");
    let path = git
        .clone_repository(&server.url(), e.parent(), "web", &Cancel::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(head(&path), "main");
    let stored = std::fs::read_to_string(path.join(".git/config")).unwrap();
    assert!(!stored.contains("s3cret") && !stored.contains("alice"));
    let refused = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", e.dir.path().join("none").as_os_str())
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .clone_repository(&server.url(), e.parent(), "web2", &Cancel::new(), |_| {})
        .await
        .unwrap_err();
    assert!(
        matches!(
            refused,
            CloneError::Net(NetError::Tls(_) | NetError::HttpAuth)
        ),
        "{refused:?}"
    );
    assert_eq!(e.listing(), ["web"]);
}

#[test]
fn names_are_proposed_from_the_url_and_checked_as_single_folder_names() {
    assert_eq!(
        repository_name_from_url("https://example.org/team/app.git"),
        "app"
    );
    assert_eq!(
        repository_name_from_url("git@example.org:team/app.git"),
        "app"
    );
    assert_eq!(
        repository_name_from_url("https://example.org/team/app/"),
        "app"
    );
    assert_eq!(repository_name_from_url("/srv/git/tool.git"), "tool");
    assert!(
        valid_directory_name("app")
            && valid_directory_name(".hidden")
            && !valid_directory_name("a/b")
    );
}
