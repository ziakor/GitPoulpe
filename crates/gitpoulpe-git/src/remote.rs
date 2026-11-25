use std::ffi::{OsStr, OsString};

use gitpoulpe_core::{Diagnostic, Remote, RemoteImpact, SessionInfo};

use crate::process::RunError;
use crate::{Cancel, Git};

const URL_LIMIT: usize = 2048;
const SCHEMES: [&str; 5] = ["https", "http", "ssh", "git", "file"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteError {
    InvalidName,
    InvalidUrl(String),
    AlreadyExists,
    NoSuchRemote,
    MultipleUrls,
    Changed,
    Refused(String),
    GitUnavailable(Diagnostic),
    GitFailed(Diagnostic),
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidName => f.write_str("nom de dépôt distant invalide pour Git"),
            Self::InvalidUrl(why) => write!(f, "URL refusée : {why}"),
            Self::AlreadyExists => {
                f.write_str("un dépôt distant porte déjà ce nom : sa configuration n'est pas remplacée")
            }
            Self::NoSuchRemote => f.write_str("ce dépôt distant n'existe pas"),
            Self::MultipleUrls => f.write_str(
                "ce dépôt distant a plusieurs URL : modifiez-les avec git config pour ne pas en perdre",
            ),
            Self::Changed => {
                f.write_str("le dépôt distant a changé depuis l'affichage, rien n'a été modifié")
            }
            Self::Refused(message) => write!(f, "Git refuse : {message}"),
            Self::GitUnavailable(d) => write!(f, "Git n'a pas pu démarrer : {d}"),
            Self::GitFailed(d) => write!(f, "Git a échoué : {d}"),
        }
    }
}

impl std::error::Error for RemoteError {}

fn failure(error: RunError) -> RemoteError {
    match error {
        RunError::Spawn(e) => RemoteError::GitUnavailable(RunError::spawn_diagnostic(&e)),
        RunError::Io(e) => RemoteError::GitFailed(RunError::spawn_diagnostic(&e)),
        RunError::Cancelled => RemoteError::Refused("cancelled".to_owned()),
        RunError::Rejected(m) => RemoteError::Refused(m),
    }
}

pub fn validate_remote_url(url: &str) -> Result<(), RemoteError> {
    let bad = |why: &str| Err(RemoteError::InvalidUrl(why.to_owned()));
    if url.is_empty() {
        return bad("elle est vide");
    }
    if url.len() > URL_LIMIT {
        return bad("elle est trop longue");
    }
    if url.starts_with('-') {
        return bad("elle commence par un tiret");
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return bad("elle contient un espace ou un caractère de contrôle");
    }
    let scheme_end = url.find(['/', ':']).filter(|i| url[*i..].starts_with(':'));
    if let Some(end) = scheme_end
        && url[end..].starts_with("::")
        && url[..end]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
    {
        return bad("les assistants de transport (ext::, fd::…) exécutent des commandes");
    }
    if let Some((scheme, rest)) = url.split_once("://") {
        if !SCHEMES.contains(&scheme.to_ascii_lowercase().as_str()) {
            return bad("protocole non pris en charge (https, http, ssh, git, file)");
        }
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        if scheme != "file" && host.is_empty() {
            return bad("l'hôte est absent");
        }
        if host.starts_with('-') || authority.starts_with('-') {
            return bad("l'hôte commence par un tiret");
        }
        return Ok(());
    }
    if url.starts_with('/') || url.starts_with("./") || url.starts_with("../") {
        return Ok(());
    }
    match url.split_once(':') {
        Some((host, path)) if !host.is_empty() && !host.contains('/') && !path.is_empty() => {
            let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
            if host.is_empty() {
                bad("l'hôte est absent")
            } else {
                Ok(())
            }
        }
        _ => bad("ni URL, ni adresse hôte:chemin, ni chemin local"),
    }
}

impl Git {
    async fn remote_run(
        &self,
        info: &SessionInfo,
        args: &[&str],
    ) -> Result<(Option<i32>, Vec<u8>, String), RemoteError> {
        let dir = info.worktree.as_deref().unwrap_or(&info.git_dir);
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let (run, out) = self
            .run_collect(dir, &refs, &Cancel::new())
            .await
            .map_err(failure)?;
        let message = String::from_utf8_lossy(&run.stderr).trim().to_owned();
        Ok((run.code, out, message))
    }

    async fn remote_ok(&self, info: &SessionInfo, args: &[&str]) -> Result<Vec<u8>, RemoteError> {
        let (code, out, message) = self.remote_run(info, args).await?;
        if code == Some(0) {
            Ok(out)
        } else {
            Err(RemoteError::Refused(message))
        }
    }

    async fn remote_lines(
        &self,
        info: &SessionInfo,
        args: &[&str],
        empty_codes: &[i32],
    ) -> Result<Vec<String>, RemoteError> {
        let (code, out, message) = self.remote_run(info, args).await?;
        match code {
            Some(0) => Ok(String::from_utf8_lossy(&out)
                .lines()
                .map(str::to_owned)
                .collect()),
            Some(c) if empty_codes.contains(&c) => Ok(Vec::new()),
            _ => Err(RemoteError::Refused(message)),
        }
    }

    pub async fn valid_remote_name(&self, info: &SessionInfo, name: &str) -> bool {
        if name.is_empty()
            || name.len() > 255
            || name == "@"
            || name.starts_with('-')
            || name.contains(['\0', '\n', '\r', '/'])
        {
            return false;
        }
        let probe = format!("refs/remotes/{name}/x");
        self.remote_ok(info, &["check-ref-format", &probe])
            .await
            .is_ok()
    }

    pub async fn list_remotes(&self, info: &SessionInfo) -> Result<Vec<Remote>, RemoteError> {
        let names = self.remote_lines(info, &["remote"], &[]).await?;
        let mut remotes = Vec::new();
        for name in names {
            let fetch_urls = self
                .remote_lines(
                    info,
                    &["config", "--get-all", &format!("remote.{name}.url")],
                    &[1],
                )
                .await?;
            let push_urls = self
                .remote_lines(
                    info,
                    &["config", "--get-all", &format!("remote.{name}.pushurl")],
                    &[1],
                )
                .await?;
            remotes.push(Remote {
                name,
                fetch_urls,
                push_urls,
            });
        }
        remotes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(remotes)
    }

    async fn find_remote(&self, info: &SessionInfo, name: &str) -> Result<Remote, RemoteError> {
        self.list_remotes(info)
            .await?
            .into_iter()
            .find(|r| r.name == name)
            .ok_or(RemoteError::NoSuchRemote)
    }

    pub async fn add_remote(
        &self,
        info: &SessionInfo,
        name: &str,
        url: &str,
    ) -> Result<Remote, RemoteError> {
        if !self.valid_remote_name(info, name).await {
            return Err(RemoteError::InvalidName);
        }
        validate_remote_url(url)?;
        if self.find_remote(info, name).await.is_ok() {
            return Err(RemoteError::AlreadyExists);
        }
        self.remote_ok(info, &["remote", "add", "--", name, url])
            .await?;
        self.find_remote(info, name).await
    }

    pub async fn set_remote_url(
        &self,
        info: &SessionInfo,
        name: &str,
        url: &str,
        expected: &[String],
    ) -> Result<Remote, RemoteError> {
        validate_remote_url(url)?;
        let current = self.find_remote(info, name).await?;
        if current.fetch_urls != expected {
            return Err(RemoteError::Changed);
        }
        if current.fetch_urls.len() != 1 {
            return Err(RemoteError::MultipleUrls);
        }
        self.remote_ok(info, &["remote", "set-url", "--", name, url])
            .await?;
        self.find_remote(info, name).await
    }

    pub async fn set_remote_push_url(
        &self,
        info: &SessionInfo,
        name: &str,
        url: Option<&str>,
        expected: &[String],
    ) -> Result<Remote, RemoteError> {
        if let Some(url) = url {
            validate_remote_url(url)?;
        }
        let current = self.find_remote(info, name).await?;
        if current.push_urls != expected {
            return Err(RemoteError::Changed);
        }
        if current.push_urls.len() > 1 {
            return Err(RemoteError::MultipleUrls);
        }
        match url {
            Some(url) => {
                self.remote_ok(info, &["remote", "set-url", "--push", "--", name, url])
                    .await?;
            }
            None if current.push_urls.is_empty() => {}
            None => {
                let key = format!("remote.{name}.pushurl");
                self.remote_ok(info, &["config", "--unset-all", &key])
                    .await?;
            }
        }
        self.find_remote(info, name).await
    }

    pub async fn rename_remote(
        &self,
        info: &SessionInfo,
        old: &str,
        new: &str,
    ) -> Result<Remote, RemoteError> {
        self.find_remote(info, old).await?;
        if !self.valid_remote_name(info, new).await {
            return Err(RemoteError::InvalidName);
        }
        if self.find_remote(info, new).await.is_ok() {
            return Err(RemoteError::AlreadyExists);
        }
        self.remote_ok(info, &["remote", "rename", "--", old, new])
            .await?;
        self.find_remote(info, new).await
    }

    pub async fn remote_impact(
        &self,
        info: &SessionInfo,
        name: &str,
    ) -> Result<RemoteImpact, RemoteError> {
        self.find_remote(info, name).await?;
        let prefix = format!("refs/remotes/{name}/");
        let refs = self
            .remote_lines(info, &["for-each-ref", "--format=%(refname)", &prefix], &[])
            .await?;
        let tracking_refs = refs.iter().filter(|r| !r.ends_with("/HEAD")).count();
        let configured = self
            .remote_lines(
                info,
                &["config", "--get-regexp", r"^branch\..*\.remote$"],
                &[1],
            )
            .await?;
        let mut upstream_of: Vec<String> = configured
            .iter()
            .filter_map(|line| {
                let (key, value) = line.split_once(' ')?;
                let branch = key.strip_prefix("branch.")?.strip_suffix(".remote")?;
                (value == name).then(|| branch.to_owned())
            })
            .collect();
        upstream_of.sort();
        Ok(RemoteImpact {
            tracking_refs,
            upstream_of,
        })
    }

    pub async fn remove_remote(
        &self,
        info: &SessionInfo,
        name: &str,
        expected: &[String],
    ) -> Result<Remote, RemoteError> {
        let current = self.find_remote(info, name).await?;
        if current.fetch_urls != expected {
            return Err(RemoteError::Changed);
        }
        self.remote_ok(info, &["remote", "remove", "--", name])
            .await?;
        Ok(current)
    }
}
