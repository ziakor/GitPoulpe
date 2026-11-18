use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use gitpoulpe_core::{Diagnostic, OpenError, SessionInfo};

use crate::process::RunError;
use crate::{Cancel, Git};

pub const DEFAULT_BRANCH: &str = "main";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InitError {
    NotFound,
    NotADirectory,
    PermissionDenied,
    InvalidBranch,
    AlreadyRepository(PathBuf),
    InsideRepository(PathBuf),
    Refused(String),
    GitUnavailable(Diagnostic),
    GitFailed(Diagnostic),
    Open(OpenError),
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("le dossier n'existe pas"),
            Self::NotADirectory => f.write_str("ce chemin n'est pas un dossier"),
            Self::PermissionDenied => {
                f.write_str("accès refusé : le dossier n'est pas modifiable, rien n'a été créé")
            }
            Self::InvalidBranch => f.write_str("nom de branche initiale invalide pour Git"),
            Self::AlreadyRepository(path) => write!(
                f,
                "ce dossier est déjà un dépôt Git ({}) : il n'est pas réinitialisé, ouvrez-le plutôt",
                path.display()
            ),
            Self::InsideRepository(root) => write!(
                f,
                "ce dossier fait déjà partie du dépôt {} : initialiser ici créerait un dépôt imbriqué",
                root.display()
            ),
            Self::Refused(message) => write!(f, "Git refuse : {message}"),
            Self::GitUnavailable(d) => write!(f, "Git n'a pas pu démarrer : {d}"),
            Self::GitFailed(d) => write!(f, "Git a échoué : {d}"),
            Self::Open(e) => write!(f, "dépôt créé mais impossible à ouvrir : {e}"),
        }
    }
}

impl std::error::Error for InitError {}

fn failure(error: RunError) -> InitError {
    match error {
        RunError::Spawn(e) => InitError::GitUnavailable(RunError::spawn_diagnostic(&e)),
        RunError::Io(e) => InitError::GitFailed(RunError::spawn_diagnostic(&e)),
        RunError::Cancelled => InitError::Refused("cancelled".to_owned()),
        RunError::Rejected(m) => InitError::Refused(m),
    }
}

fn trimmed(mut output: Vec<u8>) -> Vec<u8> {
    while matches!(output.last(), Some(b'\n' | b'\r')) {
        output.pop();
    }
    output
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Initialized {
    pub info: SessionInfo,
    pub completed_existing: bool,
}

impl Git {
    async fn init_collect(
        &self,
        dir: &Path,
        args: &[&str],
        cancel: &Cancel,
    ) -> Result<(crate::process::Run, Vec<u8>), InitError> {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        self.run_collect(dir, &refs, cancel).await.map_err(failure)
    }

    pub async fn default_branch_name(&self, dir: &Path) -> String {
        let configured = self
            .init_collect(
                dir,
                &["config", "--get", "init.defaultBranch"],
                &Cancel::new(),
            )
            .await
            .ok()
            .filter(|(run, _)| run.succeeded())
            .map(|(_, out)| String::from_utf8_lossy(&trimmed(out)).into_owned());
        match configured {
            Some(name) if self.valid_initial_branch(dir, &name).await => name,
            _ => DEFAULT_BRANCH.to_owned(),
        }
    }

    pub async fn valid_initial_branch(&self, dir: &Path, name: &str) -> bool {
        if name.is_empty()
            || name == "@"
            || name.starts_with('-')
            || name.contains(['\0', '\n', '\r'])
        {
            return false;
        }
        let full = format!("refs/heads/{name}");
        let checks: [&[&str]; 2] = [
            &["check-ref-format", "--branch", name],
            &["check-ref-format", &full],
        ];
        for args in checks {
            match self.init_collect(dir, args, &Cancel::new()).await {
                Ok((run, _)) if run.succeeded() => {}
                _ => return false,
            }
        }
        true
    }

    async fn enclosing_repository(
        &self,
        dir: &Path,
        cancel: &Cancel,
    ) -> Result<Option<PathBuf>, InitError> {
        let (run, out) = self
            .init_collect(
                dir,
                &["rev-parse", "--path-format=absolute", "--show-toplevel"],
                cancel,
            )
            .await?;
        if run.succeeded() {
            return Ok(Some(PathBuf::from(OsString::from_vec(trimmed(out)))));
        }
        let message = String::from_utf8_lossy(&run.stderr).into_owned();
        if message.contains("not a git repository") {
            return Ok(None);
        }
        if message.contains("this operation must be run in a work tree") {
            let (run, out) = self
                .init_collect(
                    dir,
                    &["rev-parse", "--path-format=absolute", "--git-dir"],
                    cancel,
                )
                .await?;
            if run.succeeded() {
                return Ok(Some(PathBuf::from(OsString::from_vec(trimmed(out)))));
            }
        }
        if message.contains("dubious ownership") || message.contains("Permission denied") {
            return Err(InitError::PermissionDenied);
        }
        Err(InitError::GitFailed(Diagnostic {
            cause: None,
            ..run.diagnostic()
        }))
    }

    pub async fn init_repository(
        &self,
        path: &Path,
        branch: &str,
        cancel: &Cancel,
    ) -> Result<Initialized, InitError> {
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(InitError::NotADirectory),
            Err(e) if e.kind() == ErrorKind::NotFound => return Err(InitError::NotFound),
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                return Err(InitError::PermissionDenied);
            }
            Err(e) => return Err(InitError::GitFailed(RunError::spawn_diagnostic(&e))),
        }
        if !self.valid_initial_branch(path, branch).await {
            return Err(InitError::InvalidBranch);
        }
        let canonical = std::fs::canonicalize(path).map_err(|e| {
            if e.kind() == ErrorKind::PermissionDenied {
                InitError::PermissionDenied
            } else {
                InitError::GitFailed(RunError::spawn_diagnostic(&e))
            }
        })?;
        if let Some(root) = self.enclosing_repository(path, cancel).await? {
            let root = std::fs::canonicalize(&root).unwrap_or(root);
            return Err(if root == canonical {
                InitError::AlreadyRepository(root)
            } else {
                InitError::InsideRepository(root)
            });
        }
        let completed_existing = canonical.join(".git").exists();
        let initial = format!("--initial-branch={branch}");
        let (run, _) = self
            .init_collect(path, &["init", "--quiet", &initial], cancel)
            .await?;
        if !run.succeeded() {
            let message = String::from_utf8_lossy(&run.stderr).into_owned();
            if message.contains("Permission denied") || message.contains("Read-only") {
                return Err(InitError::PermissionDenied);
            }
            return Err(InitError::Refused(message.trim().to_owned()));
        }
        let info = self.open(path, cancel).await.map_err(InitError::Open)?;
        Ok(Initialized {
            info,
            completed_existing,
        })
    }
}
