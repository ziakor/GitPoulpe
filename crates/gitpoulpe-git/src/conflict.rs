use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use gitpoulpe_core::{
    ConflictFile, ConflictKind, ConflictVersions, Operation, OperationKind, Profile, Resolution,
    SessionInfo, Version,
};

use crate::Git;
use crate::exec::{ExecError, ExecOut};
use crate::identity::{profile_env, signing_config};

const TEXT_LIMIT: usize = 1024 * 1024;
const BACKUP_LIMIT: u64 = 10 * 1024 * 1024;
const MARKERS: [&str; 2] = ["<<<<<<< ", ">>>>>>> "];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictSide {
    Ours,
    Theirs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConflictError {
    NoOperation,
    NoSuchConflict,
    UnsafePath,
    Symlink,
    MarkersRemain,
    NotReady(String),
    Refused(String),
    Io(String),
    Exec(ExecError),
}

impl std::fmt::Display for ConflictError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoOperation => f.write_str("aucune opération Git n'est en cours"),
            Self::NoSuchConflict => f.write_str("ce fichier n'est plus en conflit"),
            Self::UnsafePath => f.write_str("chemin refusé : il sort du dépôt"),
            Self::Symlink => f.write_str(
                "ce chemin passe par un lien symbolique : le résultat ne peut pas y être écrit",
            ),
            Self::MarkersRemain => f.write_str(
                "le résultat contient encore des marqueurs de conflit (<<<<<<< ou >>>>>>>)",
            ),
            Self::NotReady(why) => write!(f, "impossible de continuer : {why}"),
            Self::Refused(m) => write!(f, "Git refuse : {m}"),
            Self::Io(m) => write!(f, "erreur système : {m}"),
            Self::Exec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ConflictError {}

impl From<ExecError> for ConflictError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContinueOutcome {
    pub finished: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbortReport {
    pub backup: Option<PathBuf>,
    pub saved: usize,
}

fn is_safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|b| *b == 0)
}

fn has_markers(text: &str) -> bool {
    text.lines()
        .any(|line| MARKERS.iter().any(|m| line.starts_with(m)))
}

fn ensure_plain_directories(root: &Path, relative: &Path) -> Result<(), ConflictError> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(ConflictError::Symlink),
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(ConflictError::UnsafePath),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|e| ConflictError::Io(e.to_string()))?;
            }
            Err(e) => return Err(ConflictError::Io(e.to_string())),
        }
    }
    Ok(())
}

fn read_text(git_dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(git_dir.join(name))
        .ok()
        .map(|s| s.trim().to_owned())
}

fn quoted_name(message: &str) -> Option<String> {
    let start = message.find('\'')? + 1;
    let end = message[start..].find('\'')? + start;
    Some(message[start..end].to_owned())
}

impl Git {
    async fn describe_commit(&self, info: &SessionInfo, rev: &str) -> String {
        let hex = (4..=64).contains(&rev.len()) && rev.chars().all(|c| c.is_ascii_hexdigit());
        if !hex {
            return "un commit".to_owned();
        }
        self.exec(info, &["log", "-1", "--format=%h %s", rev, "--"], &[], None)
            .await
            .ok()
            .filter(ExecOut::ok)
            .map(|o| o.text())
            .unwrap_or_else(|| rev.chars().take(8).collect())
    }

    async fn current_branch_label(&self, info: &SessionInfo) -> String {
        let out = self
            .exec(info, &["symbolic-ref", "--short", "-q", "HEAD"], &[], None)
            .await
            .ok()
            .filter(ExecOut::ok)
            .map(|o| o.text());
        match out {
            Some(name) if !name.is_empty() => format!("votre branche {name}"),
            _ => "votre version (HEAD détaché)".to_owned(),
        }
    }

    pub async fn operation_state(
        &self,
        info: &SessionInfo,
    ) -> Result<Option<Operation>, ConflictError> {
        let dir = &info.git_dir;
        let ours = self.current_branch_label(info).await;
        if let Some(head) = read_text(dir, "MERGE_HEAD") {
            let message = read_text(dir, "MERGE_MSG").unwrap_or_default();
            let source = quoted_name(message.lines().next().unwrap_or_default())
                .unwrap_or_else(|| head.chars().take(8).collect());
            return Ok(Some(Operation {
                kind: OperationKind::Merge,
                ours_label: ours,
                theirs_label: format!("la branche fusionnée {source}"),
            }));
        }
        if dir.join("rebase-merge").is_dir() || dir.join("rebase-apply").is_dir() {
            let stopped = read_text(dir, "REBASE_HEAD")
                .or_else(|| read_text(dir, "rebase-merge/stopped-sha"));
            let replayed = match stopped {
                Some(sha) => self.describe_commit(info, &sha).await,
                None => "un de vos commits".to_owned(),
            };
            let onto = read_text(dir, "rebase-merge/onto")
                .or_else(|| read_text(dir, "rebase-apply/onto"))
                .map(|o| o.chars().take(8).collect::<String>())
                .unwrap_or_default();
            return Ok(Some(Operation {
                kind: OperationKind::Rebase,
                ours_label: format!("la base du rebase {onto}"),
                theirs_label: format!("votre commit rejoué {replayed}"),
            }));
        }
        if let Some(head) = read_text(dir, "CHERRY_PICK_HEAD") {
            return Ok(Some(Operation {
                kind: OperationKind::CherryPick,
                ours_label: ours,
                theirs_label: format!(
                    "le commit repris {}",
                    self.describe_commit(info, &head).await
                ),
            }));
        }
        if let Some(head) = read_text(dir, "REVERT_HEAD") {
            return Ok(Some(Operation {
                kind: OperationKind::Revert,
                ours_label: ours,
                theirs_label: format!(
                    "l'annulation du commit {}",
                    self.describe_commit(info, &head).await
                ),
            }));
        }
        if !self.unmerged(info).await?.is_empty() {
            return Ok(Some(Operation {
                kind: OperationKind::StashApply,
                ours_label: "vos fichiers actuels".to_owned(),
                theirs_label: "le stash appliqué".to_owned(),
            }));
        }
        Ok(None)
    }

    async fn unmerged(
        &self,
        info: &SessionInfo,
    ) -> Result<BTreeMap<PathBuf, BTreeMap<u8, String>>, ConflictError> {
        let out = self.exec_ok(info, &["ls-files", "-u", "-z"]).await?;
        let mut map: BTreeMap<PathBuf, BTreeMap<u8, String>> = BTreeMap::new();
        for record in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let text = String::from_utf8_lossy(record).into_owned();
            let Some((meta, path)) = text.split_once('\t') else {
                return Err(ConflictError::Refused(
                    "sortie ls-files inattendue".to_owned(),
                ));
            };
            let mut fields = meta.split(' ');
            let (_, Some(oid), Some(stage)) = (fields.next(), fields.next(), fields.next()) else {
                return Err(ConflictError::Refused(
                    "sortie ls-files inattendue".to_owned(),
                ));
            };
            let Ok(stage) = stage.parse::<u8>() else {
                continue;
            };
            map.entry(PathBuf::from(path))
                .or_default()
                .insert(stage, oid.to_owned());
        }
        Ok(map)
    }

    async fn blob(&self, info: &SessionInfo, oid: &str) -> Result<Vec<u8>, ConflictError> {
        Ok(self.exec_ok(info, &["cat-file", "blob", oid]).await?.stdout)
    }

    pub async fn list_conflicts(
        &self,
        info: &SessionInfo,
    ) -> Result<Vec<ConflictFile>, ConflictError> {
        let mut files = Vec::new();
        for (path, stages) in self.unmerged(info).await? {
            let kind = match (
                stages.contains_key(&1),
                stages.contains_key(&2),
                stages.contains_key(&3),
            ) {
                (true, true, true) => ConflictKind::BothModified,
                (false, true, true) => ConflictKind::BothAdded,
                (true, false, true) => ConflictKind::DeletedByUs,
                (true, true, false) => ConflictKind::DeletedByThem,
                (false, true, false) => ConflictKind::AddedByUs,
                (false, false, true) => ConflictKind::AddedByThem,
                _ => ConflictKind::BothDeleted,
            };
            let sample = match stages
                .get(&2)
                .or_else(|| stages.get(&3))
                .or_else(|| stages.get(&1))
            {
                Some(oid) => self.blob(info, oid).await?,
                None => Vec::new(),
            };
            files.push(ConflictFile {
                path,
                kind,
                binary: looks_binary(&sample),
            });
        }
        Ok(files)
    }

    pub async fn conflict_versions(
        &self,
        info: &SessionInfo,
        path: &Path,
    ) -> Result<ConflictVersions, ConflictError> {
        let stages = self
            .unmerged(info)
            .await?
            .remove(path)
            .ok_or(ConflictError::NoSuchConflict)?;
        let file = self
            .list_conflicts(info)
            .await?
            .into_iter()
            .find(|f| f.path == path)
            .ok_or(ConflictError::NoSuchConflict)?;
        let version = |stage: u8| {
            let oid = stages.get(&stage).cloned();
            async move {
                let Some(oid) = oid else { return Ok(None) };
                let bytes = self.blob(info, &oid).await?;
                let text = (!looks_binary(&bytes) && bytes.len() <= TEXT_LIMIT)
                    .then(|| String::from_utf8_lossy(&bytes).into_owned());
                Ok::<_, ConflictError>(Some(Version {
                    size: bytes.len() as u64,
                    text,
                    oid,
                }))
            }
        };
        let (base, ours, theirs) = (version(1).await?, version(2).await?, version(3).await?);
        let merged = info
            .worktree
            .as_deref()
            .map(|w| w.join(path))
            .and_then(|p| {
                let meta = std::fs::symlink_metadata(&p).ok()?;
                if !meta.is_file() || meta.len() as usize > TEXT_LIMIT {
                    return None;
                }
                let bytes = std::fs::read(&p).ok()?;
                (!looks_binary(&bytes)).then(|| String::from_utf8_lossy(&bytes).into_owned())
            });
        Ok(ConflictVersions {
            file,
            base,
            ours,
            theirs,
            merged,
        })
    }

    pub async fn resolve_with_side(
        &self,
        info: &SessionInfo,
        path: &Path,
        side: ConflictSide,
    ) -> Result<(), ConflictError> {
        if !is_safe_relative(path) {
            return Err(ConflictError::UnsafePath);
        }
        let stages = self
            .unmerged(info)
            .await?
            .remove(path)
            .ok_or(ConflictError::NoSuchConflict)?;
        let (stage, flag) = match side {
            ConflictSide::Ours => (2, "--ours"),
            ConflictSide::Theirs => (3, "--theirs"),
        };
        let name = path.to_string_lossy().into_owned();
        if stages.contains_key(&stage) {
            self.exec_ok(info, &["checkout", flag, "--", &name]).await?;
            self.exec_ok(info, &["add", "--", &name]).await?;
        } else {
            self.exec_ok(info, &["rm", "-q", "-f", "--", &name]).await?;
        }
        Ok(())
    }

    pub async fn resolve_with_content(
        &self,
        info: &SessionInfo,
        path: &Path,
        content: &[u8],
        allow_markers: bool,
    ) -> Result<(), ConflictError> {
        if !is_safe_relative(path) {
            return Err(ConflictError::UnsafePath);
        }
        let worktree = info.worktree.as_deref().ok_or(ExecError::NoWorktree)?;
        self.unmerged(info)
            .await?
            .remove(path)
            .ok_or(ConflictError::NoSuchConflict)?;
        if !allow_markers && has_markers(&String::from_utf8_lossy(content)) {
            return Err(ConflictError::MarkersRemain);
        }
        let root = std::fs::canonicalize(worktree).map_err(|e| ConflictError::Io(e.to_string()))?;
        let target = root.join(path);
        let parent = target
            .parent()
            .ok_or(ConflictError::UnsafePath)?
            .to_path_buf();
        ensure_plain_directories(&root, path.parent().unwrap_or(Path::new("")))?;
        let existing = std::fs::symlink_metadata(&target).ok();
        if existing
            .as_ref()
            .is_some_and(|m| m.file_type().is_symlink())
        {
            return Err(ConflictError::Symlink);
        }
        let mode = existing.map_or(0o644, |m| m.permissions().mode() & 0o777);
        let mut temp = tempfile::Builder::new()
            .prefix(".gitpoulpe-resolve-")
            .tempfile_in(&parent)
            .map_err(|e| ConflictError::Io(e.to_string()))?;
        std::io::Write::write_all(&mut temp, content)
            .and_then(|()| {
                temp.as_file()
                    .set_permissions(std::fs::Permissions::from_mode(mode))
            })
            .and_then(|()| temp.as_file().sync_all())
            .map_err(|e| ConflictError::Io(e.to_string()))?;
        temp.persist(&target)
            .map_err(|e| ConflictError::Io(e.error.to_string()))?;
        let name = path.to_string_lossy().into_owned();
        self.exec_ok(info, &["add", "--", &name]).await?;
        Ok(())
    }

    pub async fn revalidate_resolved(
        &self,
        info: &SessionInfo,
        path: &Path,
    ) -> Result<(), ConflictError> {
        if !is_safe_relative(path) {
            return Err(ConflictError::UnsafePath);
        }
        if self.unmerged(info).await?.contains_key(path) {
            return Err(ConflictError::Refused(
                "ce fichier est encore en conflit : résolvez-le d'abord".to_owned(),
            ));
        }
        let name = path.to_string_lossy().into_owned();
        self.exec_ok(info, &["add", "--", &name]).await?;
        Ok(())
    }

    pub async fn resolution_states(
        &self,
        info: &SessionInfo,
        remembered: &[PathBuf],
    ) -> Result<Vec<(PathBuf, Resolution)>, ConflictError> {
        let unmerged = self.unmerged(info).await?;
        let mut states = Vec::new();
        let mut seen: Vec<PathBuf> = Vec::new();
        for path in unmerged.keys().chain(remembered.iter()) {
            if seen.contains(path) {
                continue;
            }
            seen.push(path.clone());
            let state = if unmerged.contains_key(path) {
                Resolution::Unresolved
            } else {
                let name = path.to_string_lossy().into_owned();
                let diff = self
                    .exec(
                        info,
                        &["diff", "--quiet", "--no-ext-diff", "--", &name],
                        &[],
                        None,
                    )
                    .await?;
                if diff.code == Some(1) {
                    Resolution::Stale
                } else {
                    Resolution::Resolved
                }
            };
            states.push((path.clone(), state));
        }
        Ok(states)
    }

    pub async fn continue_operation(
        &self,
        info: &SessionInfo,
        profile: Option<&Profile>,
        remembered: &[PathBuf],
    ) -> Result<ContinueOutcome, ConflictError> {
        let operation = self
            .operation_state(info)
            .await?
            .ok_or(ConflictError::NoOperation)?;
        let states = self.resolution_states(info, remembered).await?;
        if let Some((path, state)) = states.iter().find(|(_, s)| *s != Resolution::Resolved) {
            let why = match state {
                Resolution::Unresolved => format!("{} n'est pas résolu", path.display()),
                _ => format!(
                    "{} a été modifié après sa résolution : validez-le de nouveau",
                    path.display()
                ),
            };
            return Err(ConflictError::NotReady(why));
        }
        if operation.kind == OperationKind::StashApply {
            return Err(ConflictError::NotReady(
                "il n'y a rien à continuer : une fois les fichiers résolus, ils sont simplement indexés".to_owned(),
            ));
        }
        let mut env: Vec<(OsString, OsString)> = profile.map(profile_env).unwrap_or_default();
        env.push(("GIT_EDITOR".into(), "true".into()));
        env.push(("GIT_SEQUENCE_EDITOR".into(), "true".into()));
        let config: Vec<String> = profile
            .map(signing_config)
            .unwrap_or_default()
            .into_iter()
            .flat_map(|(k, v)| ["-c".to_owned(), format!("{k}={v}")])
            .collect();
        let mut args: Vec<String> = config;
        match operation.kind {
            OperationKind::Merge => args.extend(["commit".to_owned(), "--no-edit".to_owned()]),
            other => args.extend([other.git_name().to_owned(), "--continue".to_owned()]),
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.exec(info, &refs, &env, None).await?;
        let still = self.operation_state(info).await?.is_some();
        if !out.ok() && !still {
            return Err(ConflictError::Refused(out.stderr));
        }
        if !out.ok() && self.list_conflicts(info).await?.is_empty() {
            return Err(ConflictError::Refused(out.stderr));
        }
        if !still {
            self.remove_rebase_files(info);
        }
        Ok(ContinueOutcome {
            finished: !still,
            message: if still {
                "L'opération continue : de nouveaux conflits sont à résoudre.".to_owned()
            } else {
                format!("{} terminé.", operation.kind.caption())
            },
        })
    }

    pub async fn skip_operation(
        &self,
        info: &SessionInfo,
    ) -> Result<ContinueOutcome, ConflictError> {
        let operation = self
            .operation_state(info)
            .await?
            .ok_or(ConflictError::NoOperation)?;
        if !matches!(
            operation.kind,
            OperationKind::CherryPick | OperationKind::Revert | OperationKind::Rebase
        ) {
            return Err(ConflictError::Refused(
                "cette opération ne permet pas de passer un commit".to_owned(),
            ));
        }
        let env: Vec<(OsString, OsString)> = vec![
            ("GIT_EDITOR".into(), "true".into()),
            ("GIT_SEQUENCE_EDITOR".into(), "true".into()),
        ];
        let out = self
            .exec(info, &[operation.kind.git_name(), "--skip"], &env, None)
            .await?;
        let still = self.operation_state(info).await?.is_some();
        if !out.ok() && !still {
            return Err(ConflictError::Refused(out.stderr));
        }
        if !still {
            self.remove_rebase_files(info);
        }
        Ok(ContinueOutcome {
            finished: !still,
            message: if still {
                "Commit passé ; l'opération continue avec le suivant.".to_owned()
            } else {
                format!("Commit passé ; {} terminé.", operation.kind.caption())
            },
        })
    }

    pub async fn abort_operation(
        &self,
        info: &SessionInfo,
        remembered: &[PathBuf],
    ) -> Result<AbortReport, ConflictError> {
        let operation = self
            .operation_state(info)
            .await?
            .ok_or(ConflictError::NoOperation)?;
        if operation.kind == OperationKind::StashApply {
            return Err(ConflictError::Refused(
                "l'application d'un stash ne s'abandonne pas : le stash reste intact, résolvez les fichiers ou restaurez-les vous-même".to_owned(),
            ));
        }
        let worktree = info.worktree.as_deref().ok_or(ExecError::NoWorktree)?;
        let mut paths: Vec<PathBuf> = self.unmerged(info).await?.into_keys().collect();
        for path in remembered {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let backup_root = info
            .git_dir
            .join("gitpoulpe-backup")
            .join(format!("{stamp}-{}", operation.kind.git_name()));
        let mut saved = 0;
        for path in paths.iter().filter(|p| is_safe_relative(p)) {
            let source = worktree.join(path);
            let Ok(meta) = std::fs::symlink_metadata(&source) else {
                continue;
            };
            if !meta.is_file() || meta.len() > BACKUP_LIMIT {
                continue;
            }
            let destination = backup_root.join(path);
            if let Some(parent) = destination.parent()
                && std::fs::create_dir_all(parent).is_ok()
                && std::fs::copy(&source, &destination).is_ok()
            {
                saved += 1;
            }
        }
        self.exec_ok(info, &[operation.kind.git_name(), "--abort"])
            .await?;
        self.remove_rebase_files(info);
        Ok(AbortReport {
            backup: (saved > 0).then_some(backup_root),
            saved,
        })
    }
}
