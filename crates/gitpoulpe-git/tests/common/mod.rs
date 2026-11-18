#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

pub fn git(dir: &Path, args: &[&str]) -> Vec<u8> {
    let output = git_output(dir, args);
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

pub fn git_output(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("git runs")
}

pub fn private_tempdir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(
        dir.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    dir
}

pub fn init_repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    dir
}

pub fn write(dir: &Path, name: &str, content: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

pub fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

pub fn nul_paths(bytes: &[u8]) -> Vec<PathBuf> {
    bytes
        .split(|&b| b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| PathBuf::from(OsString::from_vec(part.to_vec())))
        .collect()
}

pub fn sorted(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort();
    paths
}

pub fn tree_fingerprint(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    b"<dir>".to_vec(),
                );
                stack.push(path);
            } else if kind.is_file() {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(&path).unwrap(),
                );
            }
        }
    }
    out
}

pub struct HttpServer {
    child: std::process::Child,
    pub port: u16,
    pub dir: TempDir,
}

impl HttpServer {
    pub fn start(source: &Path, auth: Option<(&str, &str)>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        git(
            &root,
            &[
                "clone",
                "-q",
                "--bare",
                &source.display().to_string(),
                "repo.git",
            ],
        );
        let (cert, key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
        let status = Command::new("openssl")
            .args([
                "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2",
            ])
            .args(["-subj", "/CN=localhost"])
            .args(["-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1"])
            .arg("-keyout")
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .stderr(std::process::Stdio::null())
            .status()
            .expect("openssl is needed to build the test certificate");
        assert!(status.success());
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/common/git_http_server.py");
        let credentials = auth.map_or_else(|| "none".to_owned(), |(u, p)| format!("{u}:{p}"));
        let mut child = Command::new("python3")
            .arg("-I")
            .arg(script)
            .arg(&root)
            .arg(&cert)
            .arg(&key)
            .arg(&credentials)
            .arg(dir.path().join("requests.log"))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("python3 runs the test server");
        let mut line = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(child.stdout.as_mut().unwrap()),
            &mut line,
        )
        .unwrap();
        let port = line
            .trim()
            .strip_prefix("PORT ")
            .unwrap_or_else(|| panic!("test server did not start: {line:?}"))
            .parse()
            .unwrap();
        Self { child, port, dir }
    }

    pub fn url(&self) -> String {
        format!("https://127.0.0.1:{}/repo.git", self.port)
    }

    pub fn cert(&self) -> PathBuf {
        self.dir.path().join("cert.pem")
    }

    pub fn requests(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("requests.log")).unwrap_or_default()
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
