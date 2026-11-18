use std::io::Read;
use std::path::{Path, PathBuf};

use gitpoulpe_core::{LfsFile, LfsFileState, LfsPointer, LfsStatus, LfsTransfer, SessionInfo};
use sha2::{Digest, Sha256};

use crate::exec::ExecError;
use crate::net::{NetError, NetProgress};
use crate::{Cancel, Git};

const POINTER_LIMIT: usize = 1024;
const FILE_LIMIT: usize = 20_000;
const SPEC: &str = "version https://git-lfs.github.com/spec/v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LfsError {
    NotInstalled(String),
    InvalidRemote,
    AccessDenied(String),
    NotFound(String),
    Server(String),
    Net(NetError),
    Refused(String),
    Exec(ExecError),
}

impl std::fmt::Display for LfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled(how) => write!(f, "Git LFS n'est pas installé : {how}"),
            Self::InvalidRemote => f.write_str("remote ou branche invalide"),
            Self::AccessDenied(m) => write!(f, "accès refusé par le serveur LFS : {m}"),
            Self::NotFound(m) => write!(f, "dépôt ou objet LFS introuvable : {m}"),
            Self::Server(m) => write!(f, "erreur du serveur LFS (réessayez plus tard) : {m}"),
            Self::Net(e) => write!(f, "{e}"),
            Self::Refused(m) => write!(f, "Git LFS refuse : {m}"),
            Self::Exec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LfsError {}

impl From<ExecError> for LfsError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

pub fn parse_pointer(bytes: &[u8]) -> Option<LfsPointer> {
    if bytes.len() > POINTER_LIMIT {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.lines();
    if lines.next()? != SPEC {
        return None;
    }
    let (mut oid, mut size) = (None, None);
    for line in lines {
        if let Some(value) = line.strip_prefix("oid sha256:") {
            if value.len() != 64 || !value.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
                return None;
            }
            oid = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("size ") {
            size = Some(value.parse::<u64>().ok()?);
        }
    }
    Some(LfsPointer {
        oid: oid?,
        size: size?,
    })
}

fn object_path(info: &SessionInfo, oid: &str) -> PathBuf {
    info.common_dir
        .join("lfs/objects")
        .join(&oid[..2])
        .join(&oid[2..4])
        .join(oid)
}

fn read_head(path: &Path) -> Option<Vec<u8>> {
    let mut buffer = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(POINTER_LIMIT as u64 + 1)
        .read_to_end(&mut buffer)
        .ok()?;
    Some(buffer)
}

fn sha256_of(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Some(
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

impl Git {
    pub async fn lfs_status(&self, info: &SessionInfo) -> Result<LfsStatus, LfsError> {
        let out = self.exec(info, &["lfs", "version"], &[], None).await?;
        let uses_lfs = !self.lfs_tracked_paths(info).await?.is_empty();
        if out.ok() {
            let version = out.text();
            Ok(LfsStatus {
                installed: true,
                version: Some(version.clone()),
                detail: version,
                uses_lfs,
            })
        } else {
            Ok(LfsStatus {
                installed: false,
                version: None,
                detail: not_installed_help(),
                uses_lfs,
            })
        }
    }

    async fn lfs_tracked_paths(&self, info: &SessionInfo) -> Result<Vec<PathBuf>, LfsError> {
        let listed = self.exec_ok(info, &["ls-files", "-z"]).await?;
        let paths: Vec<&[u8]> = listed
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .take(FILE_LIMIT)
            .collect();
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut input = Vec::new();
        for path in &paths {
            input.extend_from_slice(path);
            input.push(0);
        }
        let attrs = self
            .exec(
                info,
                &["check-attr", "-z", "--stdin", "filter"],
                &[],
                Some(input),
            )
            .await?;
        let mut tracked = Vec::new();
        let mut fields = attrs.stdout.split(|b| *b == 0);
        while let (Some(path), Some(_), Some(value)) = (fields.next(), fields.next(), fields.next())
        {
            if value == b"lfs" {
                tracked.push(PathBuf::from(String::from_utf8_lossy(path).into_owned()));
            }
        }
        Ok(tracked)
    }

    pub async fn list_lfs_files(&self, info: &SessionInfo) -> Result<Vec<LfsFile>, LfsError> {
        let worktree = info.worktree.as_deref();
        let mut files = Vec::new();
        for path in self.lfs_tracked_paths(info).await? {
            let name = path.to_string_lossy().into_owned();
            let blob = self
                .exec(info, &["cat-file", "blob", &format!(":{name}")], &[], None)
                .await?;
            let pointer = blob.ok().then(|| parse_pointer(&blob.stdout)).flatten();
            let Some(pointer) = pointer else {
                files.push(LfsFile {
                    path,
                    pointer: None,
                    state: LfsFileState::NotConverted,
                });
                continue;
            };
            let in_store = object_path(info, &pointer.oid).is_file();
            let state = match worktree.map(|w| w.join(&path)) {
                None => {
                    if in_store {
                        LfsFileState::PointerOnly
                    } else {
                        LfsFileState::Missing
                    }
                }
                Some(full) => match std::fs::symlink_metadata(&full) {
                    Err(_) => {
                        if in_store {
                            LfsFileState::PointerOnly
                        } else {
                            LfsFileState::Missing
                        }
                    }
                    Ok(meta) => {
                        let stored = || {
                            if in_store {
                                LfsFileState::PointerOnly
                            } else {
                                LfsFileState::Missing
                            }
                        };
                        if !meta.is_file() {
                            LfsFileState::Modified
                        } else if meta.len() <= POINTER_LIMIT as u64
                            && read_head(&full)
                                .as_deref()
                                .and_then(parse_pointer)
                                .is_some()
                        {
                            stored()
                        } else if meta.len() == pointer.size
                            && sha256_of(&full).as_deref() == Some(pointer.oid.as_str())
                        {
                            LfsFileState::Present
                        } else {
                            LfsFileState::Modified
                        }
                    }
                },
            };
            files.push(LfsFile {
                path,
                pointer: Some(pointer),
                state,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    pub async fn lfs_transfer(
        &self,
        info: &SessionInfo,
        kind: LfsTransfer,
        remote: &str,
        branch: &str,
        cancel: &Cancel,
        on_progress: impl FnMut(NetProgress),
    ) -> Result<(), LfsError> {
        let status = self.lfs_status(info).await?;
        if !status.installed {
            return Err(LfsError::NotInstalled(not_installed_help()));
        }
        let valid =
            |s: &str| !s.is_empty() && !s.starts_with('-') && !s.contains(['\0', '\n', '\r']);
        if !valid(remote) || !valid(branch) {
            return Err(LfsError::InvalidRemote);
        }
        let known = self
            .list_remotes(info)
            .await
            .map_err(|e| LfsError::Refused(e.to_string()))?;
        let Some(found) = known.iter().find(|r| r.name == remote) else {
            return Err(LfsError::InvalidRemote);
        };
        let url = found.fetch_urls.first().cloned();
        let dir = info.worktree.as_deref().unwrap_or(&info.git_dir);
        let args: Vec<&str> = match kind {
            LfsTransfer::Fetch => vec!["lfs", "fetch", remote],
            LfsTransfer::Pull => vec!["lfs", "pull"],
            LfsTransfer::Push => vec!["lfs", "push", remote, branch],
        };
        let run = self
            .run_network(dir, &args, cancel, on_progress)
            .await
            .map_err(LfsError::Net)?;
        if run.code == Some(0) {
            return Ok(());
        }
        let lower = run.stderr.to_lowercase();
        let message = crate::filter_diagnostic(run.stderr.as_bytes());
        if lower.contains("authorization error")
            || lower.contains("401")
            || lower.contains("403")
            || lower.contains("forbidden")
        {
            return Err(LfsError::AccessDenied(message));
        }
        if lower.contains("not found") || lower.contains("404") {
            return Err(LfsError::NotFound(message));
        }
        if lower.contains("500")
            || lower.contains("502")
            || lower.contains("503")
            || lower.contains("server error")
        {
            return Err(LfsError::Server(message));
        }
        match self.classify_net_failure(&run.stderr, url.as_deref()).await {
            NetError::Refused(m) => Err(LfsError::Refused(m)),
            other => Err(LfsError::Net(other)),
        }
    }
}

pub fn not_installed_help() -> String {
    "GitPoulpe n'installe ni ne télécharge rien. Installez le paquet « git-lfs » avec le gestionnaire de paquets de votre système, puis exécutez une fois « git lfs install » : GitPoulpe détectera alors le support.".to_owned()
}
