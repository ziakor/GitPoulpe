use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::process::Stdio;

use gitpoulpe_core::SessionInfo;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::net::NetError;
use crate::process::RunError;
use crate::{Cancel, Git};

const KEY_TYPES: [&str; 5] = [
    "ssh-ed25519",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "ssh-rsa",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostKey {
    pub key_type: String,
    pub fingerprint: String,
    line: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCheck {
    pub branches: usize,
    pub tags: usize,
    pub head: Option<String>,
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('-')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_:".contains(c))
}

fn host_token(host: &str, port: Option<u16>) -> String {
    match port {
        Some(port) if port != 22 => format!("[{host}]:{port}"),
        _ => host.to_owned(),
    }
}

fn parse_key_line(line: &str, token: &str) -> Option<(String, String)> {
    let mut fields = line.split(' ');
    let (host, kind, key) = (fields.next()?, fields.next()?, fields.next()?);
    if fields.next().is_some()
        || host != token
        || !KEY_TYPES.contains(&kind)
        || key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))
    {
        return None;
    }
    Some((kind.to_owned(), key.to_owned()))
}

fn spawn_failure(error: &std::io::Error) -> NetError {
    NetError::GitFailed(RunError::spawn_diagnostic(error))
}

impl Git {
    fn known_hosts_file(&self) -> Option<PathBuf> {
        self.ssh.known_hosts.clone().or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".ssh/known_hosts"))
        })
    }

    pub(crate) async fn known_host_entries(&self, host: &str, port: Option<u16>) -> bool {
        let Some(file) = self.known_hosts_file() else {
            return false;
        };
        if !file.exists() || !valid_host(host) {
            return false;
        }
        let output = Command::new(&self.ssh.keygen)
            .arg("-F")
            .arg(host_token(host, port))
            .arg("-f")
            .arg(&file)
            .stdin(Stdio::null())
            .output()
            .await;
        output.is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
    }

    pub async fn scan_ssh_host(
        &self,
        host: &str,
        port: Option<u16>,
    ) -> Result<Vec<HostKey>, NetError> {
        if !valid_host(host) {
            return Err(NetError::Refused("nom d'hôte invalide".to_owned()));
        }
        let mut scan = Command::new(&self.ssh.keyscan);
        scan.args(["-T", "10"]);
        if let Some(port) = port {
            scan.arg("-p").arg(port.to_string());
        }
        let output = scan
            .arg("--")
            .arg(host)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| spawn_failure(&e))?;
        let token = host_token(host, port);
        let mut keys = Vec::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let Some((key_type, _)) = parse_key_line(line, &token) else {
                continue;
            };
            let mut child = Command::new(&self.ssh.keygen)
                .args(["-l", "-f", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| spawn_failure(&e))?;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(format!("{line}\n").as_bytes()).await;
            }
            let printed = child
                .wait_with_output()
                .await
                .map_err(|e| spawn_failure(&e))?;
            let text = String::from_utf8_lossy(&printed.stdout).into_owned();
            let Some(fingerprint) = text.split_whitespace().nth(1) else {
                continue;
            };
            if !fingerprint.starts_with("SHA256:") {
                continue;
            }
            keys.push(HostKey {
                key_type,
                fingerprint: fingerprint.to_owned(),
                line: line.to_owned(),
            });
        }
        if keys.is_empty() {
            return Err(NetError::Network(
                "aucune clé d'hôte reçue (hôte injoignable ou sans SSH)".to_owned(),
            ));
        }
        Ok(keys)
    }

    pub async fn trust_ssh_host(
        &self,
        host: &str,
        port: Option<u16>,
        fingerprint: &str,
    ) -> Result<(), NetError> {
        if self.known_host_entries(host, port).await {
            return Err(NetError::SshHostChanged {
                host: host.to_owned(),
            });
        }
        let keys = self.scan_ssh_host(host, port).await?;
        let Some(key) = keys.into_iter().find(|k| k.fingerprint == fingerprint) else {
            return Err(NetError::Refused(
                "l'empreinte présentée par l'hôte ne correspond plus à celle affichée : rien n'a été enregistré"
                    .to_owned(),
            ));
        };
        let file = self
            .known_hosts_file()
            .ok_or_else(|| NetError::Refused("répertoire personnel introuvable".to_owned()))?;
        if let Some(dir) = file.parent()
            && !dir.exists()
        {
            std::fs::create_dir_all(dir).map_err(|e| spawn_failure(&e))?;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let mut handle = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&file)
            .map_err(|e| spawn_failure(&e))?;
        let needs_newline = std::fs::read(&file)
            .map(|b| !b.is_empty() && b.last() != Some(&b'\n'))
            .unwrap_or(false);
        let mut entry = String::new();
        if needs_newline {
            entry.push('\n');
        }
        entry.push_str(&key.line);
        entry.push('\n');
        handle
            .write_all(entry.as_bytes())
            .and_then(|()| handle.sync_all())
            .map_err(|e| spawn_failure(&e))
    }

    pub async fn check_remote(
        &self,
        info: &SessionInfo,
        name: &str,
        cancel: &Cancel,
    ) -> Result<RemoteCheck, NetError> {
        let remotes = self
            .list_remotes(info)
            .await
            .map_err(|e| NetError::Refused(e.to_string()))?;
        let remote = remotes
            .into_iter()
            .find(|r| r.name == name && !r.name.starts_with('-'))
            .ok_or_else(|| NetError::Refused("ce remote n'existe pas".to_owned()))?;
        let dir = info.worktree.as_deref().unwrap_or(&info.git_dir);
        let url = remote.fetch_urls.first().map(String::as_str);
        let run = self
            .network_ok(
                dir,
                &[
                    "ls-remote",
                    "--symref",
                    "--",
                    name,
                    "HEAD",
                    "refs/heads/*",
                    "refs/tags/*",
                ],
                url,
                cancel,
                |_| {},
            )
            .await?;
        let text = String::from_utf8_lossy(&run.stdout);
        let (mut branches, mut tags, mut head) = (0, 0, None);
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("ref: ") {
                if let Some((target, "HEAD")) = rest.split_once('\t') {
                    head = target.strip_prefix("refs/heads/").map(str::to_owned);
                }
            } else if let Some((_, reference)) = line.split_once('\t') {
                if reference.ends_with("^{}") {
                    continue;
                }
                if reference.starts_with("refs/heads/") {
                    branches += 1;
                } else if reference.starts_with("refs/tags/") {
                    tags += 1;
                }
            }
        }
        Ok(RemoteCheck {
            branches,
            tags,
            head,
        })
    }
}
