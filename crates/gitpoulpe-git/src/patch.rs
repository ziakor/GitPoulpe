use std::ffi::{OsStr, OsString};

use gitpoulpe_core::SessionInfo;

use crate::Git;
use crate::cancel::Cancel;
use crate::process::RunError;

pub const PATCH_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyMode {
    WorkingTree,
    IndexAndTree,
    IndexOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchFile {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
    pub binary: bool,
    pub summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchPreview {
    pub files: Vec<PatchFile>,
    pub has_message: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchOutcome {
    Applied(Vec<String>),
    Conflicts(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchError {
    Empty,
    TooLarge,
    NotAPatch,
    Unsafe(String),
    Refused(String),
    Git(String),
    NoWorktree,
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("le fichier est vide"),
            Self::TooLarge => f.write_str("patch de plus de 8 Mio : refusé"),
            Self::NotAPatch => f.write_str("ce fichier ne contient aucun patch reconnu"),
            Self::Unsafe(path) => write!(
                f,
                "chemin dangereux dans le patch : {path} sort du dépôt ou vise le dossier interne de Git ; le patch est refusé et rien n'a été modifié"
            ),
            Self::Refused(why) => write!(
                f,
                "le patch ne s'applique pas ; rien n'a été modifié : {why}"
            ),
            Self::Git(why) => write!(f, "Git a échoué : {why}"),
            Self::NoWorktree => f.write_str("ce dépôt n'a pas de dossier de travail"),
        }
    }
}

impl std::error::Error for PatchError {}

fn unsafe_path(raw: &str) -> bool {
    let path = raw.trim().trim_matches('"');
    if path == "/dev/null" {
        return false;
    }
    let path = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);
    path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\0')
        || path.chars().nth(1) == Some(':')
        || path
            .split(['/', '\\'])
            .any(|part| part == ".." || part.eq_ignore_ascii_case(".git"))
}

fn header_paths(line: &str) -> Vec<&str> {
    if let Some(rest) = line.strip_prefix("diff --git ") {
        return rest
            .split(" b/")
            .collect::<Vec<_>>()
            .into_iter()
            .take(2)
            .collect();
    }
    for prefix in [
        "--- ",
        "+++ ",
        "rename from ",
        "rename to ",
        "copy from ",
        "copy to ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return vec![rest.split('\t').next().unwrap_or(rest)];
        }
    }
    Vec::new()
}

pub fn check_patch_paths(patch: &[u8]) -> Result<(), PatchError> {
    let text = String::from_utf8_lossy(patch);
    for line in text.lines() {
        for path in header_paths(line) {
            if unsafe_path(path) {
                return Err(PatchError::Unsafe(path.trim().to_owned()));
            }
        }
    }
    Ok(())
}

fn validate(patch: &[u8]) -> Result<(), PatchError> {
    if patch.iter().all(|b| b.is_ascii_whitespace()) {
        return Err(PatchError::Empty);
    }
    if patch.len() > PATCH_LIMIT {
        return Err(PatchError::TooLarge);
    }
    if !String::from_utf8_lossy(patch)
        .lines()
        .any(|l| l.starts_with("diff --git ") || l.starts_with("--- "))
    {
        return Err(PatchError::NotAPatch);
    }
    check_patch_paths(patch)
}

fn parse_numstat(out: &[u8]) -> Vec<PatchFile> {
    String::from_utf8_lossy(out)
        .split('\0')
        .filter(|f| !f.is_empty())
        .filter_map(|record| {
            let mut parts = record.splitn(3, '\t');
            let (added, deleted, path) = (parts.next()?, parts.next()?, parts.next()?);
            let binary = added == "-" && deleted == "-";
            Some(PatchFile {
                path: path.to_owned(),
                additions: added.parse().unwrap_or(0),
                deletions: deleted.parse().unwrap_or(0),
                binary,
                summary: None,
            })
        })
        .collect()
}

impl Git {
    async fn apply_run(
        &self,
        info: &SessionInfo,
        args: &[&str],
        patch: &[u8],
    ) -> Result<(bool, Vec<u8>, String), PatchError> {
        let dir = info.worktree.as_deref().ok_or(PatchError::NoWorktree)?;
        let owned: Vec<OsString> = args.iter().map(OsString::from).collect();
        let refs: Vec<&OsStr> = owned.iter().map(OsString::as_os_str).collect();
        let mut out = Vec::new();
        let run = self
            .run_input(
                dir,
                &refs,
                &[],
                Some(patch.to_vec()),
                &Cancel::new(),
                |chunk| {
                    out.extend_from_slice(chunk);
                    Ok(())
                },
            )
            .await
            .map_err(|e| match e {
                RunError::Spawn(e) | RunError::Io(e) => PatchError::Git(e.to_string()),
                RunError::Cancelled => PatchError::Git("annulé".to_owned()),
                RunError::Rejected(m) => PatchError::Git(m),
            })?;
        Ok((run.succeeded(), out, run.diagnostic().message))
    }

    pub async fn preview_patch(
        &self,
        info: &SessionInfo,
        patch: &[u8],
    ) -> Result<PatchPreview, PatchError> {
        validate(patch)?;
        let (ok, out, why) = self
            .apply_run(info, &["apply", "--numstat", "-z", "-"], patch)
            .await?;
        if !ok {
            return Err(PatchError::Refused(why));
        }
        let mut files = parse_numstat(&out);
        let (_, summary, _) = self
            .apply_run(info, &["apply", "--summary", "-"], patch)
            .await?;
        for line in String::from_utf8_lossy(&summary).lines() {
            let line = line.trim();
            for file in &mut files {
                if line.ends_with(&file.path) || line.contains(&format!("{} =>", file.path)) {
                    file.summary = Some(line.to_owned());
                }
            }
        }
        if files.is_empty() {
            return Err(PatchError::NotAPatch);
        }
        let has_message = String::from_utf8_lossy(patch)
            .lines()
            .take(3)
            .any(|l| l.starts_with("From "));
        Ok(PatchPreview { files, has_message })
    }

    pub async fn check_patch(
        &self,
        info: &SessionInfo,
        patch: &[u8],
        mode: ApplyMode,
    ) -> Result<(), PatchError> {
        validate(patch)?;
        let flag = match mode {
            ApplyMode::WorkingTree => None,
            ApplyMode::IndexAndTree => Some("--index"),
            ApplyMode::IndexOnly => Some("--cached"),
        };
        let mut args = vec!["apply", "--check"];
        args.extend(flag);
        args.push("-");
        let (ok, _, why) = self.apply_run(info, &args, patch).await?;
        if ok {
            Ok(())
        } else {
            Err(PatchError::Refused(why))
        }
    }

    pub async fn apply_patch(
        &self,
        info: &SessionInfo,
        patch: &[u8],
        mode: ApplyMode,
        three_way: bool,
    ) -> Result<PatchOutcome, PatchError> {
        validate(patch)?;
        let mut args = vec!["apply"];
        if three_way {
            args.push("--3way");
        } else {
            match mode {
                ApplyMode::WorkingTree => {}
                ApplyMode::IndexAndTree => args.push("--index"),
                ApplyMode::IndexOnly => args.push("--cached"),
            }
        }
        args.push("-");
        let (ok, _, why) = self.apply_run(info, &args, patch).await?;
        if !ok {
            let conflicted: Vec<String> = why
                .lines()
                .filter(|l| l.contains("with conflicts"))
                .filter_map(|l| l.split('\'').nth(1).map(str::to_owned))
                .collect();
            if three_way && !conflicted.is_empty() {
                return Ok(PatchOutcome::Conflicts(conflicted));
            }
            return Err(PatchError::Refused(why));
        }
        Ok(PatchOutcome::Applied(
            self.preview_patch_files(info, patch).await,
        ))
    }

    async fn preview_patch_files(&self, info: &SessionInfo, patch: &[u8]) -> Vec<String> {
        self.apply_run(info, &["apply", "--numstat", "-z", "-"], patch)
            .await
            .map(|(_, out, _)| parse_numstat(&out).into_iter().map(|f| f.path).collect())
            .unwrap_or_default()
    }
}
