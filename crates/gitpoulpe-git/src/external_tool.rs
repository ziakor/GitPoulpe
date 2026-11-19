use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use gitpoulpe_core::SessionInfo;
use tempfile::TempDir;

use crate::Git;

const STDERR_TAIL: usize = 400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolError {
    Empty,
    BadArguments(String),
    UnknownPlaceholder(String),
    NotFound(String),
    PermissionDenied(String),
    Failed { code: Option<i32>, detail: String },
    Io(String),
    Export(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("aucun outil configuré pour cette action"),
            Self::BadArguments(why) => write!(f, "arguments de l'outil invalides : {why}"),
            Self::UnknownPlaceholder(name) => {
                write!(f, "variable inconnue dans les arguments : {{{name}}}")
            }
            Self::NotFound(program) => write!(f, "exécutable introuvable : {program}"),
            Self::PermissionDenied(program) => write!(f, "exécution refusée : {program}"),
            Self::Failed {
                code: Some(code),
                detail,
            } if detail.is_empty() => {
                write!(f, "l'outil s'est terminé avec le code {code}")
            }
            Self::Failed {
                code: Some(code),
                detail,
            } => {
                write!(f, "l'outil s'est terminé avec le code {code} : {detail}")
            }
            Self::Failed { code: None, .. } => {
                f.write_str("l'outil a été interrompu par un signal")
            }
            Self::Io(why) => write!(f, "impossible de lancer l'outil : {why}"),
            Self::Export(why) => {
                write!(f, "impossible de préparer les versions du fichier : {why}")
            }
        }
    }
}

impl std::error::Error for ToolError {}

pub fn split_args(text: &str) -> Result<Vec<String>, ToolError> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(other) => current.push(other),
                        None => {
                            return Err(ToolError::BadArguments(
                                "apostrophe non fermée".to_owned(),
                            ));
                        }
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(escaped @ ('"' | '\\')) => current.push(escaped),
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                            None => {
                                return Err(ToolError::BadArguments(
                                    "guillemet non fermé".to_owned(),
                                ));
                            }
                        },
                        Some(other) => current.push(other),
                        None => {
                            return Err(ToolError::BadArguments("guillemet non fermé".to_owned()));
                        }
                    }
                }
            }
            other => {
                started = true;
                current.push(other);
            }
        }
    }
    if started {
        args.push(current);
    }
    Ok(args)
}

fn placeholders(argument: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = argument;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                found.push(after[..end].to_owned());
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    found
}

pub fn expand_args(template: &str, vars: &[(&str, &str)]) -> Result<Vec<String>, ToolError> {
    let mut expanded = Vec::new();
    for argument in split_args(template)? {
        for name in placeholders(&argument) {
            if !vars.iter().any(|(known, _)| *known == name) {
                return Err(ToolError::UnknownPlaceholder(name));
            }
        }
        let mut value = String::new();
        let mut rest = argument.as_str();
        while let Some(start) = rest.find('{') {
            value.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let Some(end) = after.find('}') else {
                value.push_str(&rest[start..]);
                rest = "";
                break;
            };
            let name = &after[..end];
            if let Some((_, replacement)) = vars.iter().find(|(known, _)| *known == name) {
                value.push_str(replacement);
            }
            rest = &after[end + 1..];
        }
        value.push_str(rest);
        expanded.push(value);
    }
    Ok(expanded)
}

pub async fn run_tool(program: &str, args: &[String], cwd: &Path) -> Result<(), ToolError> {
    if program.trim().is_empty() {
        return Err(ToolError::Empty);
    }
    let output = tokio::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ToolError::NotFound(program.to_owned()),
            std::io::ErrorKind::PermissionDenied => ToolError::PermissionDenied(program.to_owned()),
            _ => ToolError::Io(e.to_string()),
        })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let trimmed = stderr.trim();
    let start = trimmed.len().saturating_sub(STDERR_TAIL);
    let start = (start..=trimmed.len())
        .find(|i| trimmed.is_char_boundary(*i))
        .unwrap_or(trimmed.len());
    Err(ToolError::Failed {
        code: output.status.code(),
        detail: trimmed[start..].to_owned(),
    })
}

pub struct ToolFiles {
    dir: TempDir,
}

impl ToolFiles {
    pub fn new() -> Result<Self, ToolError> {
        let dir = tempfile::Builder::new()
            .prefix("gitpoulpe-tool-")
            .tempdir()
            .map_err(|e| ToolError::Export(e.to_string()))?;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(|e| ToolError::Export(e.to_string()))?;
        Ok(Self { dir })
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    fn slot(&self, label: &str, name: &str) -> Result<PathBuf, ToolError> {
        let base = Path::new(name)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "file".to_owned());
        let dir = self.dir.path().join(label);
        std::fs::create_dir_all(&dir).map_err(|e| ToolError::Export(e.to_string()))?;
        Ok(dir.join(base))
    }
}

impl Git {
    pub async fn export_version(
        &self,
        info: &SessionInfo,
        spec: &str,
        files: &ToolFiles,
        label: &str,
        name: &str,
    ) -> Result<PathBuf, ToolError> {
        let target = files.slot(label, name)?;
        let (probe, _) = self
            .collect_in(info, &["cat-file", "-e", spec])
            .await
            .map_err(|e| ToolError::Export(e.to_string()))?;
        let bytes = if probe.succeeded() {
            let (run, bytes) = self
                .collect_in(info, &["cat-file", "blob", spec])
                .await
                .map_err(|e| ToolError::Export(e.to_string()))?;
            if !run.succeeded() {
                return Err(ToolError::Export(format!("{spec} n'est pas lisible")));
            }
            bytes
        } else {
            Vec::new()
        };
        std::fs::write(&target, bytes).map_err(|e| ToolError::Export(e.to_string()))?;
        Ok(target)
    }
}
