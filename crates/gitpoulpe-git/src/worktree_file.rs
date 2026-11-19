use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use gitpoulpe_core::SessionInfo;

pub const EDIT_LIMIT: u64 = 512 * 1024;
const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileToken {
    len: u64,
    hash: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeFile {
    pub path: String,
    pub text: String,
    pub line_ending: LineEnding,
    pub bom: bool,
    pub token: FileToken,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    InvalidPath,
    NoWorktree,
    NotFound,
    NotRegular,
    Binary,
    UnsupportedEncoding,
    MixedLineEndings,
    TooLarge,
    ExternalChange,
    PermissionDenied(String),
    Io(String),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath => f.write_str("chemin refusé : il doit désigner un fichier du dépôt"),
            Self::NoWorktree => f.write_str("ce dépôt n'a pas de dossier de travail"),
            Self::NotFound => f.write_str("le fichier n'existe pas (ou plus) dans le dossier de travail"),
            Self::NotRegular => f.write_str("ce n'est pas un fichier ordinaire (lien symbolique, dossier ou fichier spécial) : édition refusée"),
            Self::Binary => f.write_str("fichier binaire : édition refusée"),
            Self::UnsupportedEncoding => f.write_str("encodage non pris en charge (seul UTF-8 est édité) : le fichier n'est pas modifié"),
            Self::MixedLineEndings => f.write_str("fins de ligne mélangées : édition refusée pour ne pas les modifier sans que vous le sachiez"),
            Self::TooLarge => f.write_str("fichier de plus de 512 Kio : trop volumineux pour l'éditeur intégré"),
            Self::ExternalChange => f.write_str("le fichier a changé sur le disque depuis son ouverture"),
            Self::PermissionDenied(path) => write!(f, "accès refusé en écriture sur {path} : rien n'a été écrit"),
            Self::Io(message) => write!(f, "erreur d'écriture : {message}"),
        }
    }
}

impl std::error::Error for EditError {}

fn hash_of(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn token_of(bytes: &[u8]) -> FileToken {
    FileToken {
        len: bytes.len() as u64,
        hash: hash_of(bytes),
    }
}

fn io_error(error: std::io::Error, path: &Path) -> EditError {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => {
            EditError::PermissionDenied(path.display().to_string())
        }
        std::io::ErrorKind::NotFound => EditError::NotFound,
        _ => EditError::Io(error.to_string()),
    }
}

fn resolve(info: &SessionInfo, relative: &str) -> Result<PathBuf, EditError> {
    let root = info.worktree.as_deref().ok_or(EditError::NoWorktree)?;
    let rel = Path::new(relative);
    let mut components = rel.components().peekable();
    if relative.is_empty() || relative.contains('\0') || rel.is_absolute() {
        return Err(EditError::InvalidPath);
    }
    if let Some(Component::Normal(first)) = components.peek()
        && first.to_string_lossy().eq_ignore_ascii_case(".git")
    {
        return Err(EditError::InvalidPath);
    }
    if !components.all(|c| matches!(c, Component::Normal(_))) {
        return Err(EditError::InvalidPath);
    }
    let target = root.join(rel);
    let parent = target.parent().ok_or(EditError::InvalidPath)?;
    let canonical_root = root.canonicalize().map_err(|e| io_error(e, root))?;
    let canonical_parent = parent.canonicalize().map_err(|e| io_error(e, parent))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(EditError::InvalidPath);
    }
    Ok(target)
}

fn regular(target: &Path) -> Result<std::fs::Metadata, EditError> {
    let meta = std::fs::symlink_metadata(target).map_err(|e| io_error(e, target))?;
    if !meta.file_type().is_file() {
        return Err(EditError::NotRegular);
    }
    Ok(meta)
}

fn line_ending_of(text: &str) -> Result<LineEnding, EditError> {
    let bytes = text.as_bytes();
    let (mut crlf, mut lf, mut cr) = (0usize, 0usize, 0usize);
    for (i, b) in bytes.iter().enumerate() {
        match b {
            b'\n' if i > 0 && bytes[i - 1] == b'\r' => crlf += 1,
            b'\n' => lf += 1,
            b'\r' if bytes.get(i + 1) != Some(&b'\n') => cr += 1,
            _ => {}
        }
    }
    if cr > 0 || (crlf > 0 && lf > 0) {
        return Err(EditError::MixedLineEndings);
    }
    Ok(if crlf > 0 {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    })
}

pub fn read_worktree_file(info: &SessionInfo, relative: &str) -> Result<WorktreeFile, EditError> {
    let target = resolve(info, relative)?;
    let meta = regular(&target)?;
    if meta.len() > EDIT_LIMIT {
        return Err(EditError::TooLarge);
    }
    let bytes = std::fs::read(&target).map_err(|e| io_error(e, &target))?;
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(EditError::UnsupportedEncoding);
    }
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Err(EditError::Binary);
    }
    let bom = bytes.starts_with(BOM);
    let body = if bom { &bytes[BOM.len()..] } else { &bytes[..] };
    let text = std::str::from_utf8(body)
        .map_err(|_| EditError::UnsupportedEncoding)?
        .to_owned();
    let line_ending = line_ending_of(&text)?;
    let text = if line_ending == LineEnding::Crlf {
        text.replace("\r\n", "\n")
    } else {
        text
    };
    Ok(WorktreeFile {
        path: relative.to_owned(),
        text,
        line_ending,
        bom,
        token: token_of(&bytes),
    })
}

pub fn save_worktree_file(
    info: &SessionInfo,
    original: &WorktreeFile,
    text: &str,
    overwrite_external: bool,
) -> Result<WorktreeFile, EditError> {
    let target = resolve(info, &original.path)?;
    let meta = regular(&target)?;
    if meta.permissions().readonly() {
        return Err(EditError::PermissionDenied(target.display().to_string()));
    }
    let current = std::fs::read(&target).map_err(|e| io_error(e, &target))?;
    if !overwrite_external && token_of(&current) != original.token {
        return Err(EditError::ExternalChange);
    }
    let mut bytes = Vec::with_capacity(text.len() + 3);
    if original.bom {
        bytes.extend_from_slice(BOM);
    }
    if original.line_ending == LineEnding::Crlf {
        bytes.extend_from_slice(text.replace('\n', "\r\n").as_bytes());
    } else {
        bytes.extend_from_slice(text.as_bytes());
    }
    let directory = target.parent().ok_or(EditError::InvalidPath)?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = directory.join(format!(
        ".{name}.gitpoulpe-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::set_permissions(&temporary, meta.permissions())?;
        std::fs::rename(&temporary, &target)
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(io_error(error, &target));
    }
    Ok(WorktreeFile {
        path: original.path.clone(),
        text: text.to_owned(),
        line_ending: original.line_ending,
        bom: original.bom,
        token: token_of(&bytes),
    })
}
