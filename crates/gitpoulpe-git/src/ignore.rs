use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use gitpoulpe_core::{
    Diagnostic, IgnoreReason, IgnoreScope, IgnoreSource, SessionInfo, is_plain_relative,
};

use crate::process::RunError;
use crate::{Cancel, Git};

pub const MAX_IGNORED_ENTRIES: usize = 5_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IgnoreError {
    NoWorktree,
    InvalidPath,
    InvalidRule,
    Symlink(PathBuf),
    UnknownScope(String),
    AlreadyPresent,
    NotFound,
    Changed,
    Io(String),
    GitUnavailable(Diagnostic),
    GitFailed(Diagnostic),
}

impl std::fmt::Display for IgnoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoWorktree => f.write_str("le dépôt n'a pas de répertoire de travail"),
            Self::InvalidPath => f.write_str("chemin invalide"),
            Self::InvalidRule => {
                f.write_str("règle invalide (vide, ligne multiple ou commentaire)")
            }
            Self::Symlink(path) => write!(
                f,
                "{} est un lien symbolique : GitPoulpe ne l'écrit pas",
                path.display()
            ),
            Self::UnknownScope(m) => write!(f, "fichier d'exclusion introuvable : {m}"),
            Self::AlreadyPresent => f.write_str("cette règle existe déjà dans ce fichier"),
            Self::NotFound => f.write_str("cette règle n'existe pas dans ce fichier"),
            Self::Changed => {
                f.write_str("le fichier a changé pendant l'écriture, rien n'a été modifié")
            }
            Self::Io(m) => write!(f, "écriture impossible : {m}"),
            Self::GitUnavailable(d) => write!(f, "Git n'a pas pu démarrer : {d}"),
            Self::GitFailed(d) => write!(f, "Git a échoué : {d}"),
        }
    }
}

impl std::error::Error for IgnoreError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IgnoredList {
    pub paths: Vec<PathBuf>,
    pub truncated: bool,
}

fn failure(error: RunError) -> IgnoreError {
    match error {
        RunError::Spawn(e) => IgnoreError::GitUnavailable(RunError::spawn_diagnostic(&e)),
        RunError::Io(e) => IgnoreError::GitFailed(RunError::spawn_diagnostic(&e)),
        RunError::Cancelled => IgnoreError::Io("cancelled".to_owned()),
        RunError::Rejected(m) => IgnoreError::Io(m),
    }
}

fn io_error(error: &std::io::Error) -> IgnoreError {
    IgnoreError::Io(crate::filter_diagnostic(error.to_string().as_bytes()))
}

impl Git {
    async fn ignore_run(
        &self,
        info: &SessionInfo,
        args: &[&OsStr],
    ) -> Result<(crate::process::Run, Vec<u8>), IgnoreError> {
        let worktree = info.worktree.as_deref().ok_or(IgnoreError::NoWorktree)?;
        self.run_collect(worktree, args, &Cancel::new())
            .await
            .map_err(failure)
    }

    pub async fn ignore_files(
        &self,
        info: &SessionInfo,
    ) -> Result<(PathBuf, Option<PathBuf>), IgnoreError> {
        let worktree = info.worktree.as_deref().ok_or(IgnoreError::NoWorktree)?;
        let args: Vec<&OsStr> = ["rev-parse", "--git-path", "info/exclude"]
            .iter()
            .map(OsStr::new)
            .collect();
        let (run, out) = self.ignore_run(info, &args).await?;
        if !run.succeeded() {
            return Err(IgnoreError::GitFailed(run.diagnostic()));
        }
        let text = out.strip_suffix(b"\n").unwrap_or(&out);
        let exclude = worktree.join(OsStr::from_bytes(text));

        let args: Vec<&OsStr> = ["config", "--global", "--path", "--get", "core.excludesFile"]
            .iter()
            .map(OsStr::new)
            .collect();
        let (run, out) = self.ignore_run(info, &args).await?;
        let configured = (run.succeeded() && !out.is_empty()).then(|| {
            let text = out.strip_suffix(b"\n").unwrap_or(&out);
            PathBuf::from(OsString::from_vec(text.to_vec()))
        });
        let global = configured.or_else(|| {
            let base = std::env::var_os("XDG_CONFIG_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
            Some(base.join("git").join("ignore"))
        });
        Ok((exclude, global))
    }

    pub async fn ignore_file(
        &self,
        info: &SessionInfo,
        scope: &IgnoreScope,
    ) -> Result<PathBuf, IgnoreError> {
        let worktree = info.worktree.as_deref().ok_or(IgnoreError::NoWorktree)?;
        match scope {
            IgnoreScope::Directory(dir) => {
                if !dir.as_os_str().is_empty() && !is_plain_relative(dir) {
                    return Err(IgnoreError::InvalidPath);
                }
                let folder = worktree.join(dir);
                let real = std::fs::canonicalize(&folder).map_err(|e| io_error(&e))?;
                let root = std::fs::canonicalize(worktree).map_err(|e| io_error(&e))?;
                if !real.starts_with(&root) {
                    return Err(IgnoreError::InvalidPath);
                }
                Ok(real.join(".gitignore"))
            }
            IgnoreScope::Exclude => {
                let exclude = self.ignore_files(info).await?.0;
                let expected = std::fs::canonicalize(info.common_dir.join("info"))
                    .map_err(|e| io_error(&e))?;
                let parent = exclude.parent().ok_or(IgnoreError::InvalidPath)?;
                let inside = std::fs::canonicalize(parent).is_ok_and(|p| p == expected)
                    && exclude.file_name() == Some(OsStr::new("exclude"));
                if inside {
                    Ok(exclude)
                } else {
                    Err(IgnoreError::InvalidPath)
                }
            }
            IgnoreScope::Global => self
                .ignore_files(info)
                .await?
                .1
                .ok_or_else(|| IgnoreError::UnknownScope("core.excludesFile".to_owned())),
        }
    }

    pub async fn ignore_reason(
        &self,
        info: &SessionInfo,
        path: &Path,
    ) -> Result<Option<IgnoreReason>, IgnoreError> {
        if !is_plain_relative(path) {
            return Err(IgnoreError::InvalidPath);
        }
        let worktree = info.worktree.as_deref().ok_or(IgnoreError::NoWorktree)?;
        let tracked = {
            let args = [
                OsStr::new("--literal-pathspecs"),
                OsStr::new("ls-files"),
                OsStr::new("-z"),
                OsStr::new("--"),
                path.as_os_str(),
            ];
            let (run, out) = self.ignore_run(info, &args).await?;
            if !run.succeeded() {
                return Err(IgnoreError::GitFailed(run.diagnostic()));
            }
            !out.is_empty()
        };
        let args = [
            OsStr::new("check-ignore"),
            OsStr::new("-v"),
            OsStr::new("-z"),
            OsStr::new("--no-index"),
            OsStr::new("--stdin"),
        ];
        let mut input = path.as_os_str().as_bytes().to_vec();
        input.push(0);
        let mut out = Vec::new();
        let run = self
            .run_input(worktree, &args, &[], Some(input), &Cancel::new(), |chunk| {
                out.extend_from_slice(chunk);
                Ok(())
            })
            .await
            .map_err(failure)?;
        if !matches!(run.code, Some(0 | 1)) {
            return Err(IgnoreError::GitFailed(run.diagnostic()));
        }
        let mut fields = out.split(|b| *b == 0);
        let (Some(source), Some(line), Some(pattern)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return Ok(None);
        };
        if source.is_empty() && pattern.is_empty() {
            return Ok(None);
        }
        let pattern_text = String::from_utf8_lossy(pattern).into_owned();
        let negated = pattern_text.starts_with('!');
        let line: u32 = String::from_utf8_lossy(line).parse().unwrap_or(0);
        let source = self
            .classify_source(info, worktree, Path::new(OsStr::from_bytes(source)))
            .await;
        Ok(Some(IgnoreReason {
            source,
            line,
            pattern: pattern_text,
            negated,
            tracked,
        }))
    }

    async fn classify_source(
        &self,
        info: &SessionInfo,
        worktree: &Path,
        source: &Path,
    ) -> IgnoreSource {
        let text = source.to_string_lossy().into_owned();
        let absolute = if source.is_absolute() {
            source.to_path_buf()
        } else {
            worktree.join(source)
        };
        let real = std::fs::canonicalize(&absolute).unwrap_or(absolute);
        if let Ok((exclude, global)) = self.ignore_files(info).await {
            let same = |other: &Path| std::fs::canonicalize(other).is_ok_and(|o| o == real);
            if same(&exclude) {
                return IgnoreSource::Scope(IgnoreScope::Exclude);
            }
            if global.as_deref().is_some_and(same) {
                return IgnoreSource::Scope(IgnoreScope::Global);
            }
        }
        if real.file_name() == Some(OsStr::new(".gitignore"))
            && let Ok(root) = std::fs::canonicalize(worktree)
            && let Ok(inside) = real.strip_prefix(&root)
        {
            let dir = inside.parent().unwrap_or(Path::new("")).to_path_buf();
            return IgnoreSource::Scope(IgnoreScope::Directory(dir));
        }
        IgnoreSource::Other(text)
    }

    pub async fn list_ignored(&self, info: &SessionInfo) -> Result<IgnoredList, IgnoreError> {
        let args: Vec<&OsStr> = [
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ]
        .iter()
        .map(OsStr::new)
        .collect();
        let worktree = info.worktree.as_deref().ok_or(IgnoreError::NoWorktree)?;
        let mut out = Vec::new();
        let run = self
            .run(worktree, &args, &Cancel::new(), |chunk| {
                if out.len() > 8 * 1024 * 1024 {
                    return Err("too much output".to_owned());
                }
                out.extend_from_slice(chunk);
                Ok(())
            })
            .await
            .map_err(failure)?;
        if !run.succeeded() {
            return Err(IgnoreError::GitFailed(run.diagnostic()));
        }
        let mut paths: Vec<PathBuf> = out
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| PathBuf::from(OsString::from_vec(p.to_vec())))
            .collect();
        paths.sort();
        let truncated = paths.len() > MAX_IGNORED_ENTRIES;
        paths.truncate(MAX_IGNORED_ENTRIES);
        Ok(IgnoredList { paths, truncated })
    }

    pub async fn add_ignore_rule(
        &self,
        info: &SessionInfo,
        scope: &IgnoreScope,
        rule: &str,
    ) -> Result<PathBuf, IgnoreError> {
        check_rule(rule)?;
        let file = self.ignore_file(info, scope).await?;
        edit_file(&file, scope != &IgnoreScope::Global, |lines| {
            if lines.iter().any(|l| same_rule(l, rule)) {
                return Err(IgnoreError::AlreadyPresent);
            }
            lines.push(rule.as_bytes().to_vec());
            Ok(())
        })?;
        Ok(file)
    }

    pub async fn remove_ignore_rule(
        &self,
        info: &SessionInfo,
        scope: &IgnoreScope,
        rule: &str,
    ) -> Result<PathBuf, IgnoreError> {
        check_rule(rule)?;
        let file = self.ignore_file(info, scope).await?;
        edit_file(&file, scope != &IgnoreScope::Global, |lines| {
            let before = lines.len();
            lines.retain(|l| !same_rule(l, rule));
            if lines.len() == before {
                return Err(IgnoreError::NotFound);
            }
            Ok(())
        })?;
        Ok(file)
    }
}

fn same_rule(line: &[u8], rule: &str) -> bool {
    line.strip_suffix(b"\r").unwrap_or(line) == rule.as_bytes()
}

fn check_rule(rule: &str) -> Result<(), IgnoreError> {
    if rule.trim().is_empty()
        || rule.contains(['\n', '\r', '\0'])
        || rule.starts_with('#')
        || rule != rule.trim_start()
    {
        return Err(IgnoreError::InvalidRule);
    }
    Ok(())
}

fn split_lines(bytes: &[u8]) -> Vec<Vec<u8>> {
    let body = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    if bytes.is_empty() {
        return Vec::new();
    }
    body.split(|b| *b == b'\n').map(<[u8]>::to_vec).collect()
}

fn read_current(path: &Path) -> Result<Vec<u8>, IgnoreError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(io_error(&e)),
    }
}

fn refuse_symlink(path: &Path) -> Result<(), IgnoreError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(IgnoreError::Symlink(path.to_path_buf())),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_error(&e)),
    }
}

fn edit_file(
    path: &Path,
    protect_from_symlink: bool,
    change: impl FnOnce(&mut Vec<Vec<u8>>) -> Result<(), IgnoreError>,
) -> Result<(), IgnoreError> {
    if protect_from_symlink {
        refuse_symlink(path)?;
    }
    let target = if protect_from_symlink {
        path.to_path_buf()
    } else {
        match std::fs::canonicalize(path) {
            Ok(real) => real,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
            Err(e) => return Err(io_error(&e)),
        }
    };
    let before = read_current(&target)?;
    let mut lines = split_lines(&before);
    change(&mut lines)?;
    let mut after = Vec::new();
    for line in &lines {
        after.extend_from_slice(line);
        after.push(b'\n');
    }
    let parent = target.parent().ok_or(IgnoreError::InvalidPath)?;
    std::fs::create_dir_all(parent).map_err(|e| io_error(&e))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| io_error(&e))?;
    temp.write_all(&after).map_err(|e| io_error(&e))?;
    temp.as_file().sync_all().map_err(|e| io_error(&e))?;
    if let Ok(meta) = std::fs::metadata(&target) {
        let _ = std::fs::set_permissions(temp.path(), meta.permissions());
    }
    if protect_from_symlink {
        refuse_symlink(&target)?;
    }
    if read_current(&target)? != before {
        return Err(IgnoreError::Changed);
    }
    temp.persist(&target).map_err(|e| io_error(&e.error))?;
    Ok(())
}
