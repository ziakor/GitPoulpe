#![allow(dead_code)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub struct FakeApi {
    child: Child,
    pub port: u16,
    log: PathBuf,
    _dir: tempfile::TempDir,
}

impl FakeApi {
    pub fn start(script: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("requests.log");
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/common")
            .join(script);
        let mut child = Command::new("python3")
            .arg("-I")
            .arg(path)
            .arg(&log)
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

    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn requests(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for FakeApi {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
