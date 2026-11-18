mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{Cancel, Git, NetError, SshTools, parse_progress, remote_host};

const FAKE_SSH: &str = include_str!("common/fake_ssh.sh");

struct Env {
    dir: tempfile::TempDir,
    repo: tempfile::TempDir,
    _source: tempfile::TempDir,
    git: Git,
    info: SessionInfo,
}

impl Env {
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
    fn mode(&self, mode: &str) {
        std::fs::write(self.path("mode"), mode).unwrap();
    }
    fn log(&self) -> String {
        std::fs::read_to_string(self.path("log")).unwrap_or_default()
    }
    fn known_hosts(&self) -> PathBuf {
        self.path("known_hosts")
    }
}

fn tools(dir: &Path) -> SshTools {
    SshTools {
        keyscan: dir.join("fake-keyscan").into(),
        keygen: "ssh-keygen".into(),
        known_hosts: Some(dir.join("known_hosts")),
    }
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fakessh");
    std::fs::write(&script, FAKE_SSH).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.path().join("mode"), "ok").unwrap();
    let source = init_repo();
    write(source.path(), "a.txt", "one\n");
    commit_all(source.path(), "first");
    run_git(source.path(), &["branch", "dev"]);
    run_git(source.path(), &["tag", "v1"]);
    let repo = init_repo();
    write(repo.path(), "b.txt", "x\n");
    commit_all(repo.path(), "local");
    let url = format!("ssh://git@fakehost{}", source.path().display());
    run_git(repo.path(), &["remote", "add", "origin", &url]);
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_SSH_COMMAND", script.as_os_str())
        .with_env("FAKESSH_DIR", dir.path().as_os_str())
        .with_ssh_tools(tools(dir.path()));
    let info = git.open(repo.path(), &Cancel::new()).await.unwrap();
    Env {
        dir,
        repo,
        _source: source,
        git,
        info,
    }
}

#[tokio::test]
async fn a_read_through_ssh_works_with_the_configured_command_and_keeps_host_verification_on() {
    let e = env().await;
    let check = e
        .git
        .check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap();
    assert_eq!((check.branches, check.tags), (2, 1));
    assert_eq!(check.head.as_deref(), Some("main"));
    let log = e.log();
    assert!(log.contains("BatchMode=yes"), "{log}");
    assert!(!log.contains("StrictHostKeyChecking"), "{log}");
    assert!(!log.contains("UserKnownHostsFile=/dev/null"), "{log}");
}

#[tokio::test]
async fn a_repository_configured_ssh_command_never_runs() {
    let e = env().await;
    let marker = e.path("pwned");
    run_git(
        e.repo.path(),
        &[
            "config",
            "core.sshCommand",
            &format!("touch {}; false", marker.display()),
        ],
    );
    e.git
        .check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap();
    assert!(!marker.exists());
    assert!(e.log().contains("fakehost"));
}

#[tokio::test]
async fn an_unknown_host_is_a_decision_and_a_changed_one_is_refused() {
    let e = env().await;
    e.mode("unknown");
    let error = e
        .git
        .check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        NetError::SshHostUnknown {
            host: "fakehost".to_owned(),
            port: None
        }
    );
    std::fs::write(e.known_hosts(), "fakehost ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOldKeyOldKeyOldKeyOldKeyOldKeyOldKeyOldKey\n").unwrap();
    let error = e
        .git
        .check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        NetError::SshHostChanged {
            host: "fakehost".to_owned()
        }
    );
    e.mode("changed");
    std::fs::remove_file(e.known_hosts()).unwrap();
    let error = e
        .git
        .check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap_err();
    assert!(
        matches!(error, NetError::SshHostChanged { .. }),
        "{error:?}"
    );
    assert!(!e.log().contains("StrictHostKeyChecking"));
}

#[tokio::test]
async fn a_refusal_explains_whether_an_agent_was_reachable() {
    let e = env().await;
    e.mode("denied");
    let socket = e.path("agent.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let with_agent = e.git.clone().with_env("SSH_AUTH_SOCK", socket.as_os_str());
    assert_eq!(
        with_agent
            .check_remote(&e.info, "origin", &Cancel::new())
            .await
            .unwrap_err(),
        NetError::SshAccessDenied {
            agent_available: true
        }
    );
    let without = e
        .git
        .clone()
        .with_env("SSH_AUTH_SOCK", e.path("missing.sock").as_os_str());
    assert_eq!(
        without
            .check_remote(&e.info, "origin", &Cancel::new())
            .await
            .unwrap_err(),
        NetError::SshAccessDenied {
            agent_available: false
        }
    );
}

#[tokio::test]
async fn an_unreachable_host_is_a_network_error() {
    let e = env().await;
    e.mode("offline");
    let error = e
        .git
        .check_remote(&e.info, "origin", &Cancel::new())
        .await
        .unwrap_err();
    assert!(matches!(error, NetError::Network(_)), "{error:?}");
}

fn make_key(dir: &Path) -> (String, String) {
    let key = dir.join("hostkey");
    let status = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "", "-f"])
        .arg(&key)
        .status()
        .unwrap();
    assert!(status.success());
    let public = std::fs::read_to_string(dir.join("hostkey.pub")).unwrap();
    let mut parts = public.split_whitespace();
    let (kind, blob) = (parts.next().unwrap(), parts.next().unwrap());
    let fingerprint = std::process::Command::new("ssh-keygen")
        .arg("-lf")
        .arg(dir.join("hostkey.pub"))
        .output()
        .unwrap();
    let fingerprint = String::from_utf8_lossy(&fingerprint.stdout)
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    (format!("{kind} {blob}"), fingerprint)
}

fn fake_keyscan(dir: &Path, lines: &str) {
    let script = dir.join("fake-keyscan");
    std::fs::write(&script, format!("#!/bin/sh\ncat <<'EOF'\n{lines}\nEOF\n")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn trusting_a_host_needs_the_fingerprint_that_was_shown_and_writes_only_that_key() {
    let e = env().await;
    let (key, fingerprint) = make_key(e.dir.path());
    fake_keyscan(
        e.dir.path(),
        &format!(
            "# fakehost:22 SSH-2.0\n@cert-authority * {key}\nother.example {key}\nfakehost {key}\nfakehost ssh-ed25519 AAAA second-field"
        ),
    );
    let keys = e.git.scan_ssh_host("fakehost", None).await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].fingerprint, fingerprint);
    assert_eq!(keys[0].key_type, "ssh-ed25519");
    assert!(e.git.scan_ssh_host("-oProxyCommand=x", None).await.is_err());
    let wrong = e
        .git
        .trust_ssh_host("fakehost", None, "SHA256:notTheShownOne")
        .await
        .unwrap_err();
    assert!(matches!(wrong, NetError::Refused(_)));
    assert!(!e.known_hosts().exists());
    e.git
        .trust_ssh_host("fakehost", None, &fingerprint)
        .await
        .unwrap();
    let written = std::fs::read_to_string(e.known_hosts()).unwrap();
    assert_eq!(written, format!("fakehost {key}\n"));
    assert_eq!(
        std::fs::metadata(e.known_hosts())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let again = e
        .git
        .trust_ssh_host("fakehost", None, &fingerprint)
        .await
        .unwrap_err();
    assert_eq!(
        again,
        NetError::SshHostChanged {
            host: "fakehost".to_owned()
        }
    );
    assert_eq!(std::fs::read_to_string(e.known_hosts()).unwrap(), written);
}

#[tokio::test]
async fn an_existing_known_hosts_file_is_appended_to_never_rewritten() {
    let e = env().await;
    let (key, fingerprint) = make_key(e.dir.path());
    fake_keyscan(e.dir.path(), &format!("[fakehost]:2222 {key}"));
    std::fs::write(e.known_hosts(), "keep.example ssh-ed25519 AAAAkeep").unwrap();
    e.git
        .trust_ssh_host("fakehost", Some(2222), &fingerprint)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(e.known_hosts()).unwrap(),
        format!("keep.example ssh-ed25519 AAAAkeep\n[fakehost]:2222 {key}\n")
    );
}

#[tokio::test]
async fn cancelling_stops_git_and_every_process_it_started() {
    let e = env().await;
    e.mode("hang");
    let cancel = Cancel::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        canceller.cancel();
    });
    let started = std::time::Instant::now();
    let error = e
        .git
        .check_remote(&e.info, "origin", &cancel)
        .await
        .unwrap_err();
    assert_eq!(error, NetError::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(10));
    let pid = std::fs::read_to_string(e.path("sleeper"))
        .unwrap()
        .trim()
        .to_owned();
    let mut alive = true;
    for _ in 0..40 {
        alive = std::path::Path::new(&format!("/proc/{pid}")).exists()
            && !std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map(|s| s.contains(") Z"))
                .unwrap_or(true);
        if !alive {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!alive, "the child process survived the cancellation");
}

#[tokio::test]
async fn a_cancel_requested_before_the_start_runs_nothing() {
    let e = env().await;
    let cancel = Cancel::new();
    cancel.cancel();
    assert_eq!(
        e.git
            .check_remote(&e.info, "origin", &cancel)
            .await
            .unwrap_err(),
        NetError::Cancelled
    );
    assert!(e.log().is_empty());
}

#[test]
fn progress_lines_are_recognised_and_other_lines_are_not() {
    let p = parse_progress("Receiving objects:  45% (450/1000), 1.2 MiB | 1.0 MiB/s").unwrap();
    assert_eq!((p.phase.as_str(), p.percent), ("Receiving objects", 45));
    let p = parse_progress("remote: Counting objects: 100% (12/12), done.").unwrap();
    assert_eq!((p.phase.as_str(), p.percent), ("Counting objects", 100));
    assert!(parse_progress("fatal: unable to access 'https://x/': failed").is_none());
    assert!(parse_progress("remote: Total 3 (delta 0)").is_none());
    assert!(parse_progress("Cloning into 'x'...").is_none());
}

#[test]
fn the_ssh_host_is_extracted_from_every_url_form() {
    assert_eq!(
        remote_host("ssh://git@example.org/a.git"),
        Some(("example.org".into(), None))
    );
    assert_eq!(
        remote_host("ssh://git@example.org:2222/a.git"),
        Some(("example.org".into(), Some(2222)))
    );
    assert_eq!(
        remote_host("git@example.org:team/a.git"),
        Some(("example.org".into(), None))
    );
    assert_eq!(
        remote_host("ssh://[::1]:2200/a.git"),
        Some(("::1".into(), Some(2200)))
    );
    assert_eq!(remote_host("https://example.org/a.git"), None);
    assert_eq!(remote_host("/srv/repo.git"), None);
}
