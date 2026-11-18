use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use gitpoulpe_core::url_has_secret;

use crate::net::{NetError, NetProgress};
use crate::remote::validate_remote_url;
use crate::{Cancel, Git, RemoteError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloneError {
    InvalidUrl(String),
    SecretInUrl,
    InvalidName,
    ParentMissing,
    DestinationNotEmpty(PathBuf),
    NotWritable,
    Net(NetError),
    Io(String),
}

impl std::fmt::Display for CloneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl(why) => write!(f, "URL refusée : {why}"),
            Self::SecretInUrl => f.write_str(
                "l'URL contient des identifiants : retirez-les et utilisez un credential helper, ils seraient sinon écrits dans la configuration du dépôt",
            ),
            Self::InvalidName => f.write_str("nom de dossier invalide"),
            Self::ParentMissing => f.write_str("le dossier parent n'existe pas"),
            Self::DestinationNotEmpty(path) => write!(
                f,
                "{} existe déjà et n'est pas vide : rien n'a été cloné",
                path.display()
            ),
            Self::NotWritable => {
                f.write_str("accès refusé : le dossier n'est pas modifiable, rien n'a été créé")
            }
            Self::Net(e) => write!(f, "{e}"),
            Self::Io(m) => write!(f, "erreur système : {m}"),
        }
    }
}

impl std::error::Error for CloneError {}

pub fn repository_name_from_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let last = trimmed
        .rsplit(['/', ':'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".git");
    last.chars()
        .filter(|c| !c.is_control() && *c != '/')
        .collect::<String>()
        .trim_matches('.')
        .to_owned()
}

pub fn valid_directory_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.starts_with('-')
        && !name.contains(['/', '\0'])
        && !name.chars().any(char::is_control)
}

struct Partial {
    path: PathBuf,
    armed: bool,
}

impl Drop for Partial {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn io_failure(error: &std::io::Error) -> CloneError {
    if error.kind() == ErrorKind::PermissionDenied {
        CloneError::NotWritable
    } else {
        CloneError::Io(error.to_string())
    }
}

fn create_partial(parent: &Path, name: &str) -> Result<Partial, CloneError> {
    for attempt in 0..100u32 {
        let path = parent.join(format!(".{name}.partial-{}-{attempt}", std::process::id()));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(Partial { path, armed: true }),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io_failure(&e)),
        }
    }
    Err(CloneError::Io("no free temporary name".to_owned()))
}

impl Git {
    pub async fn clone_repository(
        &self,
        url: &str,
        parent: &Path,
        name: &str,
        cancel: &Cancel,
        on_progress: impl FnMut(NetProgress),
    ) -> Result<PathBuf, CloneError> {
        validate_remote_url(url).map_err(|e| match e {
            RemoteError::InvalidUrl(why) => CloneError::InvalidUrl(why),
            other => CloneError::InvalidUrl(other.to_string()),
        })?;
        if url_has_secret(url) {
            return Err(CloneError::SecretInUrl);
        }
        if !valid_directory_name(name) {
            return Err(CloneError::InvalidName);
        }
        match std::fs::metadata(parent) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(CloneError::ParentMissing),
            Err(e) if e.kind() == ErrorKind::NotFound => return Err(CloneError::ParentMissing),
            Err(e) => return Err(io_failure(&e)),
        }
        let parent = std::fs::canonicalize(parent).map_err(|e| io_failure(&e))?;
        let destination = parent.join(name);
        match std::fs::symlink_metadata(&destination) {
            Ok(meta) => {
                let empty = meta.is_dir()
                    && std::fs::read_dir(&destination)
                        .map(|mut entries| entries.next().is_none())
                        .unwrap_or(false);
                if !empty {
                    return Err(CloneError::DestinationNotEmpty(destination));
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(io_failure(&e)),
        }
        let mut partial = create_partial(&parent, name)?;
        let target = partial.path.display().to_string();
        self.network_ok(
            &parent,
            &["clone", "--progress", "--", url, &target],
            Some(url),
            cancel,
            on_progress,
        )
        .await
        .map_err(CloneError::Net)?;
        if cancel.is_cancelled() {
            return Err(CloneError::Net(NetError::Cancelled));
        }
        if destination.exists() {
            std::fs::remove_dir(&destination)
                .map_err(|_| CloneError::DestinationNotEmpty(destination.clone()))?;
        }
        std::fs::rename(&partial.path, &destination).map_err(|e| io_failure(&e))?;
        partial.armed = false;
        Ok(destination)
    }
}
