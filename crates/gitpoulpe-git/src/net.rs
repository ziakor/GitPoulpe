use std::ffi::{OsStr, OsString};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use gitpoulpe_core::Diagnostic;
use tokio::io::AsyncReadExt;

use crate::diagnostics::filter_diagnostic;
use crate::process::RunError;
use crate::{Cancel, Git};

const MAX_STDOUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;
const TERMINATE_GRACE: Duration = Duration::from_secs(2);
const SSH_OPTIONS: &str =
    "-o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=15 -o ServerAliveCountMax=4";
const SCRUBBED_NET_ENV: [&str; 3] = ["GIT_SSL_NO_VERIFY", "GIT_PROXY_COMMAND", "GIT_SSH"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetProgress {
    pub phase: String,
    pub percent: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetError {
    Cancelled,
    SshHostUnknown { host: String, port: Option<u16> },
    SshHostChanged { host: String },
    SshAccessDenied { agent_available: bool },
    HttpAuth,
    Tls(String),
    Network(String),
    NotFound,
    Refused(String),
    GitUnavailable(Diagnostic),
    GitFailed(Diagnostic),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("opération annulée"),
            Self::SshHostUnknown { host, .. } => write!(
                f,
                "hôte SSH inconnu ({host}) : vérifiez son empreinte avant de lui faire confiance"
            ),
            Self::SshHostChanged { host } => write!(
                f,
                "l'identité de l'hôte SSH {host} a changé : connexion refusée, c'est peut-être une interception"
            ),
            Self::SshAccessDenied { agent_available } => {
                if *agent_available {
                    f.write_str("accès SSH refusé : aucune clé de l'agent n'est acceptée par le serveur")
                } else {
                    f.write_str(
                        "accès SSH refusé : aucun agent SSH n'est joignable ; lancez ssh-agent et ajoutez votre clé avec ssh-add (une clé protégée par phrase secrète doit passer par l'agent)",
                    )
                }
            }
            Self::HttpAuth => f.write_str(
                "authentification HTTPS refusée ou absente : configurez un credential helper (git config --global credential.helper)",
            ),
            Self::Tls(m) => write!(
                f,
                "erreur de certificat TLS, la vérification n'est pas désactivée : {m}"
            ),
            Self::Network(m) => write!(f, "réseau indisponible : {m}"),
            Self::NotFound => f.write_str("dépôt distant introuvable ou inaccessible"),
            Self::Refused(m) => write!(f, "Git refuse : {m}"),
            Self::GitUnavailable(d) => write!(f, "Git n'a pas pu démarrer : {d}"),
            Self::GitFailed(d) => write!(f, "Git a échoué : {d}"),
        }
    }
}

impl std::error::Error for NetError {}

#[derive(Clone, Debug)]
pub struct SshTools {
    pub keyscan: OsString,
    pub keygen: OsString,
    pub known_hosts: Option<PathBuf>,
}

impl Default for SshTools {
    fn default() -> Self {
        Self {
            keyscan: OsString::from("ssh-keyscan"),
            keygen: OsString::from("ssh-keygen"),
            known_hosts: None,
        }
    }
}

pub struct NetRun {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

pub fn parse_progress(segment: &str) -> Option<NetProgress> {
    let segment = segment.trim();
    let segment = segment.strip_prefix("remote: ").unwrap_or(segment);
    let (phase, rest) = segment.split_once(':')?;
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() || !rest.trim_start()[digits.len()..].starts_with('%') {
        return None;
    }
    Some(NetProgress {
        phase: phase.trim().to_owned(),
        percent: digits.parse::<u8>().ok()?.min(100),
    })
}

pub fn remote_host(url: &str) -> Option<(String, Option<u16>)> {
    let (authority, ssh_like) = if let Some(rest) = url.strip_prefix("ssh://") {
        (rest.split('/').next()?, true)
    } else if url.contains("://") {
        return None;
    } else {
        let (head, _) = url.split_once(':')?;
        if head.contains('/') {
            return None;
        }
        (head, false)
    };
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if ssh_like
        && let Some((host, port)) = hostport.rsplit_once(':')
        && let Ok(port) = port.parse::<u16>()
    {
        return Some((host.trim_matches(['[', ']']).to_owned(), Some(port)));
    }
    let host = hostport.trim_matches(['[', ']']);
    (!host.is_empty()).then(|| (host.to_owned(), None))
}

fn agent_socket(extra: &[(OsString, OsString)]) -> Option<PathBuf> {
    extra
        .iter()
        .rev()
        .find(|(k, _)| k == "SSH_AUTH_SOCK")
        .map(|(_, v)| PathBuf::from(v))
        .or_else(|| std::env::var_os("SSH_AUTH_SOCK").map(PathBuf::from))
}

impl Git {
    pub fn with_ssh_tools(mut self, tools: SshTools) -> Self {
        self.ssh = tools;
        self
    }

    fn agent_available(&self) -> bool {
        agent_socket(self.extra_env()).is_some_and(|p| p.exists())
    }

    async fn trusted_config_values(&self, dir: &Path) -> Vec<(String, String, String)> {
        let args = ["config", "--list", "--show-scope", "-z"].map(OsString::from);
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let Ok((run, out)) = self.run_collect(dir, &refs, &Cancel::new()).await else {
            return Vec::new();
        };
        if !run.succeeded() {
            return Vec::new();
        }
        let text = String::from_utf8_lossy(&out).into_owned();
        let mut entries = Vec::new();
        let mut parts = text.split('\0');
        while let (Some(scope), Some(pair)) = (parts.next(), parts.next()) {
            let (key, value) = pair.split_once('\n').unwrap_or((pair, ""));
            entries.push((scope.to_owned(), key.to_owned(), value.to_owned()));
        }
        entries
    }

    async fn net_config_args(&self, dir: &Path) -> Vec<OsString> {
        let mut args: Vec<OsString> = Vec::new();
        let mut push = |pair: String| {
            args.push(OsString::from("-c"));
            args.push(OsString::from(pair));
        };
        for protocol in ["https", "http", "ssh", "git"] {
            push(format!("protocol.{protocol}.allow=always"));
        }
        push("protocol.file.allow=user".to_owned());
        push("http.sslVerify=true".to_owned());
        push("http.lowSpeedLimit=1000".to_owned());
        push("http.lowSpeedTime=60".to_owned());
        let entries = self.trusted_config_values(dir).await;
        let guarded = |key: &str| {
            key.starts_with("credential.") && key.ends_with(".helper")
                || key == "credential.helper"
                || key == "core.askpass"
        };
        let mut reset: Vec<&str> = Vec::new();
        for (scope, key, _) in &entries {
            if matches!(scope.as_str(), "local" | "worktree")
                && guarded(key)
                && !reset.contains(&key.as_str())
            {
                reset.push(key);
            }
        }
        for key in reset {
            push(format!("{key}="));
            for (scope, k, value) in &entries {
                if matches!(scope.as_str(), "system" | "global") && k == key {
                    push(format!("{key}={value}"));
                }
            }
        }
        args
    }

    async fn ssh_command(&self, dir: &Path) -> OsString {
        let from_env = self
            .extra_env()
            .iter()
            .rev()
            .find(|(k, _)| k == "GIT_SSH_COMMAND")
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var_os("GIT_SSH_COMMAND"));
        let base = match from_env {
            Some(command) => command,
            None => {
                let entries = self.trusted_config_values(dir).await;
                entries
                    .into_iter()
                    .rev()
                    .find(|(scope, key, _)| {
                        matches!(scope.as_str(), "system" | "global") && key == "core.sshcommand"
                    })
                    .map_or_else(|| OsString::from("ssh"), |(_, _, v)| OsString::from(v))
            }
        };
        let mut command = base;
        command.push(" ");
        command.push(SSH_OPTIONS);
        command
    }

    pub(crate) async fn run_network(
        &self,
        dir: &Path,
        args: &[&str],
        cancel: &Cancel,
        mut on_progress: impl FnMut(NetProgress),
    ) -> Result<NetRun, NetError> {
        if cancel.is_cancelled() {
            return Err(NetError::Cancelled);
        }
        let mut full: Vec<OsString> = self.net_config_args(dir).await;
        full.extend(args.iter().map(OsString::from));
        let refs: Vec<&OsStr> = full.iter().map(OsString::as_os_str).collect();
        let ssh = self.ssh_command(dir).await;
        let mut command = self.command(dir, &refs, &[], false);
        for name in SCRUBBED_NET_ENV {
            command.env_remove(name);
        }
        command.env("GIT_SSH_COMMAND", ssh);
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|e| NetError::GitUnavailable(RunError::spawn_diagnostic(&e)))?;
        let pid = child.id();
        let mut stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let mut out = Vec::new();
        let mut diagnostic = String::new();
        let mut pending = Vec::new();
        let mut out_buf = vec![0u8; 16 * 1024];
        let mut err_buf = vec![0u8; 4 * 1024];
        let (mut out_open, mut err_open) = (true, true);
        let mut cancelled = false;
        while out_open || err_open {
            tokio::select! {
                biased;
                () = cancel.cancelled() => { cancelled = true; break; }
                read = stdout.read(&mut out_buf), if out_open => match read {
                    Ok(0) | Err(_) => out_open = false,
                    Ok(n) => {
                        let room = MAX_STDOUT_BYTES.saturating_sub(out.len());
                        out.extend_from_slice(&out_buf[..n.min(room)]);
                    }
                },
                read = stderr.read(&mut err_buf), if err_open => match read {
                    Ok(0) | Err(_) => err_open = false,
                    Ok(n) => {
                        for byte in &err_buf[..n] {
                            if matches!(byte, b'\n' | b'\r') {
                                let segment = String::from_utf8_lossy(&pending).into_owned();
                                pending.clear();
                                if segment.trim().is_empty() {
                                    continue;
                                }
                                match parse_progress(&segment) {
                                    Some(progress) => on_progress(progress),
                                    None if diagnostic.len() < MAX_DIAGNOSTIC_BYTES => {
                                        diagnostic.push_str(&segment);
                                        diagnostic.push('\n');
                                    }
                                    None => {}
                                }
                            } else if pending.len() < 8 * 1024 {
                                pending.push(*byte);
                            }
                        }
                    }
                },
            }
        }
        if cancelled {
            terminate_group(&mut child, pid).await;
            return Err(NetError::Cancelled);
        }
        if !pending.is_empty() {
            diagnostic.push_str(&String::from_utf8_lossy(&pending));
        }
        let status = child
            .wait()
            .await
            .map_err(|e| NetError::GitFailed(RunError::spawn_diagnostic(&e)))?;
        Ok(NetRun {
            code: status.code().or(status.signal().map(|s| 128 + s)),
            stdout: out,
            stderr: diagnostic,
        })
    }

    pub(crate) async fn classify_net_failure(&self, stderr: &str, url: Option<&str>) -> NetError {
        let lower = stderr.to_lowercase();
        let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
        let host = url.and_then(remote_host);
        if has(&["remote host identification has changed", "offending key"]) {
            return NetError::SshHostChanged {
                host: host.map_or_else(String::new, |(h, _)| h),
            };
        }
        if has(&["host key verification failed"]) {
            let Some((name, port)) = host else {
                return NetError::Refused(filter_diagnostic(stderr.as_bytes()));
            };
            return if self.known_host_entries(&name, port).await {
                NetError::SshHostChanged { host: name }
            } else {
                NetError::SshHostUnknown { host: name, port }
            };
        }
        if has(&[
            "permission denied (publickey",
            "permission denied (password",
            "no supported authentication methods",
            "too many authentication failures",
        ]) {
            return NetError::SshAccessDenied {
                agent_available: self.agent_available(),
            };
        }
        if has(&[
            "ssl certificate problem",
            "server certificate verification failed",
            "certificate verify failed",
            "unable to get local issuer certificate",
            "self-signed certificate",
            "self signed certificate",
            "tls connect error",
        ]) {
            return NetError::Tls(filter_diagnostic(stderr.as_bytes()));
        }
        if has(&[
            "authentication failed",
            "could not read username",
            "could not read password",
            "terminal prompts disabled",
            "invalid username or password",
            "returned error: 401",
            "returned error: 403",
        ]) {
            return NetError::HttpAuth;
        }
        if has(&[
            "could not resolve",
            "name or service not known",
            "connection refused",
            "connection timed out",
            "operation timed out",
            "network is unreachable",
            "no route to host",
            "failed to connect",
            "connection reset",
            "unable to access",
        ]) && !has(&["returned error: 404"])
        {
            return NetError::Network(filter_diagnostic(stderr.as_bytes()));
        }
        if has(&[
            "repository not found",
            "does not appear to be a git repository",
            "returned error: 404",
        ]) {
            return NetError::NotFound;
        }
        NetError::Refused(filter_diagnostic(stderr.as_bytes()))
    }

    pub(crate) async fn network_ok(
        &self,
        dir: &Path,
        args: &[&str],
        url: Option<&str>,
        cancel: &Cancel,
        on_progress: impl FnMut(NetProgress),
    ) -> Result<NetRun, NetError> {
        let run = self.run_network(dir, args, cancel, on_progress).await?;
        if run.code == Some(0) {
            return Ok(run);
        }
        Err(self.classify_net_failure(&run.stderr, url).await)
    }
}

async fn terminate_group(child: &mut tokio::process::Child, pid: Option<u32>) {
    let Some(pid) = pid else {
        let _ = child.start_kill();
        let _ = child.wait().await;
        return;
    };
    let signal = |name: &str| {
        let _ = std::process::Command::new("kill")
            .args([name, "--", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    };
    signal("-TERM");
    if tokio::time::timeout(TERMINATE_GRACE, child.wait())
        .await
        .is_err()
    {
        signal("-KILL");
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    signal("-KILL");
}
