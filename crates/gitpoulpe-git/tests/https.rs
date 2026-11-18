mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use common::{HttpServer, commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{Cancel, Git, NetError};

struct Env {
    dir: tempfile::TempDir,
    repo: tempfile::TempDir,
    server: HttpServer,
    _source: tempfile::TempDir,
    info: SessionInfo,
}

impl Env {
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.dir.path().join(name)
    }
    fn git(&self, config: &str) -> Git {
        let file = self.path("gitconfig");
        std::fs::write(&file, config).unwrap();
        Git::new()
            .with_env("GIT_CONFIG_GLOBAL", file.as_os_str())
            .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
    }
    fn trusting(&self, extra: &str) -> String {
        format!(
            "[http]\n\tsslCAInfo = {}\n{extra}",
            self.server.cert().display()
        )
    }
}

fn helper(dir: &Path, user: &str, pass: &str) -> String {
    let script = dir.join("helper.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\necho \"$1\" >> \"{}/helper.log\"\necho username={user}\necho password={pass}\n", dir.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    format!("[credential]\n\thelper = {}\n", script.display())
}

async fn env(auth: Option<(&str, &str)>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let source = init_repo();
    write(source.path(), "a.txt", "one\n");
    commit_all(source.path(), "first");
    run_git(source.path(), &["branch", "dev"]);
    run_git(source.path(), &["tag", "v1"]);
    let server = HttpServer::start(source.path(), auth);
    let repo = init_repo();
    write(repo.path(), "b.txt", "x\n");
    commit_all(repo.path(), "local");
    run_git(repo.path(), &["remote", "add", "origin", &server.url()]);
    let info = Git::new().open(repo.path(), &Cancel::new()).await.unwrap();
    Env {
        dir,
        repo,
        server,
        _source: source,
        info,
    }
}

#[tokio::test]
async fn a_helper_supplies_the_credentials_and_the_url_never_holds_a_secret() {
    let e = env(Some(("alice", "s3cret"))).await;
    let git = e.git(&format!(
        "{}{}",
        e.trusting(""),
        helper(e.dir.path(), "alice", "s3cret")
    ));
    let check = git
        .check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap();
    assert_eq!((check.branches, check.tags), (2, 1));
    assert!(e.server.requests().contains("auth=yes"));
    let url = String::from_utf8_lossy(&run_git(e.repo.path(), &["remote", "get-url", "origin"]))
        .into_owned();
    assert!(!url.contains("alice") && !url.contains("s3cret"));
    assert!(
        !std::fs::read_to_string(e.repo.path().join(".git/config"))
            .unwrap()
            .contains("s3cret")
    );
}

#[tokio::test]
async fn a_credential_helper_written_in_the_repository_never_runs() {
    let e = env(Some(("alice", "s3cret"))).await;
    let marker = e.path("pwned");
    run_git(
        e.repo.path(),
        &[
            "config",
            "credential.helper",
            &format!(
                "!touch {}; echo username=alice; echo password=s3cret; true",
                marker.display()
            ),
        ],
    );
    run_git(
        e.repo.path(),
        &[
            "config",
            "core.askPass",
            &format!("touch {}", marker.display()),
        ],
    );
    let git = e.git(&format!(
        "{}{}",
        e.trusting(""),
        helper(e.dir.path(), "alice", "s3cret")
    ));
    git.check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap();
    assert!(!marker.exists());
    assert!(
        std::fs::read_to_string(e.path("helper.log"))
            .unwrap()
            .contains("get")
    );
}

#[tokio::test]
async fn without_any_credentials_the_operation_ends_cleanly_instead_of_prompting() {
    let e = env(Some(("alice", "s3cret"))).await;
    let git = e.git(&e.trusting(""));
    let started = std::time::Instant::now();
    let error = git
        .check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap_err();
    assert_eq!(error, NetError::HttpAuth);
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[tokio::test]
async fn refused_credentials_end_cleanly() {
    let e = env(Some(("alice", "s3cret"))).await;
    let git = e.git(&format!(
        "{}{}",
        e.trusting(""),
        helper(e.dir.path(), "alice", "wrong")
    ));
    let error = git
        .check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap_err();
    assert_eq!(error, NetError::HttpAuth);
}

#[tokio::test]
async fn cancelling_an_open_credential_request_stops_the_helper_too() {
    let e = env(Some(("alice", "s3cret"))).await;
    let script = e.path("slow.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nsleep 60 &\necho $! > \"{}/sleeper\"\nwait\n",
            e.dir.path().display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let git = e.git(&format!(
        "{}[credential]\n\thelper = {}\n",
        e.trusting(""),
        script.display()
    ));
    let cancel = Cancel::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        canceller.cancel();
    });
    let error = git
        .check_remote(&e.info, "origin", &cancel)
        .await
        .unwrap_err();
    assert_eq!(error, NetError::Cancelled);
    let pid = std::fs::read_to_string(e.path("sleeper"))
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
    assert!(!alive, "the helper survived the cancellation");
}

#[tokio::test]
async fn an_untrusted_certificate_is_a_tls_error_and_verification_stays_on() {
    let e = env(None).await;
    for config in ["", "[http]\n\tsslVerify = false\n"] {
        let git = e.git(config).with_env("GIT_SSL_NO_VERIFY", "1");
        run_git(e.repo.path(), &["config", "http.sslVerify", "false"]);
        let error = git
            .check_remote(&e.info, "origin", &Cancel::new())
            .await
            .unwrap_err();
        assert!(matches!(error, NetError::Tls(_)), "{error:?}");
    }
    let trusted = e.git(&e.trusting(""));
    trusted
        .check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_closed_port_is_a_network_error_and_messages_hide_embedded_credentials() {
    let e = env(None).await;
    run_git(
        e.repo.path(),
        &[
            "remote",
            "set-url",
            "origin",
            "https://bob:tok3n@127.0.0.1:1/repo.git",
        ],
    );
    let git = e.git("");
    let error = git
        .check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap_err();
    let NetError::Network(message) = &error else {
        panic!("{error:?}");
    };
    assert!(
        !message.contains("tok3n") && !message.contains("bob"),
        "{message}"
    );
    assert!(!error.to_string().contains("tok3n"));
}

#[tokio::test]
async fn a_missing_repository_is_reported_as_not_found() {
    let e = env(None).await;
    run_git(
        e.repo.path(),
        &[
            "remote",
            "set-url",
            "origin",
            &format!("https://127.0.0.1:{}/nothing.git", e.server.port),
        ],
    );
    let error = e
        .git(&e.trusting(""))
        .check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap_err();
    assert!(
        matches!(error, NetError::NotFound | NetError::Refused(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn an_askpass_program_written_in_the_repository_never_runs() {
    let e = env(Some(("alice", "s3cret"))).await;
    let marker = e.path("asked");
    let script = e.path("askpass.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\ntouch {}\necho alice\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    run_git(
        e.repo.path(),
        &["config", "core.askPass", &script.display().to_string()],
    );
    let error = e
        .git(&e.trusting(""))
        .check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap_err();
    assert_eq!(error, NetError::HttpAuth);
    assert!(!marker.exists());
}
