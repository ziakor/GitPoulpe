use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use gitpoulpe_hosting::{GithubError, GithubHost, HttpError, MemoryStore, connect_github};

struct TlsServer {
    child: Child,
    port: u16,
    log: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl TlsServer {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("cert.pem");
        let key = dir.path().join("key.pem");
        let made = Command::new("openssl")
            .args([
                "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
            ])
            .args(["-subj", "/CN=127.0.0.1"])
            .arg("-keyout")
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .output()
            .unwrap();
        assert!(made.status.success());
        let log = dir.path().join("log");
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/common/fake_tls.py");
        let mut child = Command::new("python3")
            .arg("-I")
            .arg(script)
            .arg(&log)
            .arg(&cert)
            .arg(&key)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        Self {
            child,
            port: line.trim().parse().unwrap(),
            log,
            _dir: dir,
        }
    }
}

impl Drop for TlsServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn enterprise(base: &str) -> GithubHost {
    GithubHost::enterprise(base).unwrap()
}

#[tokio::test]
async fn an_untrusted_certificate_is_refused_before_any_request_and_stores_nothing() {
    let server = TlsServer::start();
    let store = MemoryStore::default();
    let host = enterprise(&format!("https://127.0.0.1:{}", server.port));
    let error = connect_github(&store, &host, "secret-token")
        .await
        .unwrap_err();
    assert!(
        matches!(error, GithubError::Http(HttpError::Tls(_))),
        "{error:?}"
    );
    assert!(!error.to_string().contains("secret-token"));
    assert!(
        std::fs::read_to_string(&server.log)
            .unwrap_or_default()
            .is_empty()
    );
    assert!(store.keys().is_empty());
}

#[tokio::test]
async fn an_unreachable_instance_reports_itself_and_never_reaches_another_host() {
    let store = MemoryStore::default();
    let host = enterprise("http://127.0.0.1:1");
    let error = connect_github(&store, &host, "secret-token")
        .await
        .unwrap_err();
    assert!(
        matches!(error, GithubError::Http(HttpError::Offline(_))),
        "{error:?}"
    );
    assert!(!error.to_string().contains("secret-token"));
    assert!(store.keys().is_empty());
    assert_eq!(host.api, "http://127.0.0.1:1/api/v3");
}
