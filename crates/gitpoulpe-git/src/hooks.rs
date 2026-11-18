use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use gitpoulpe_core::{HookInfo, HooksView, SessionInfo};

use crate::Git;
use crate::exec::ExecError;

pub const KNOWN_HOOKS: [&str; 28] = [
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-receive",
    "update",
    "proc-receive",
    "post-receive",
    "post-update",
    "reference-transaction",
    "push-to-checkout",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "fsmonitor-watchman",
    "post-index-change",
    "p4-changelist",
    "p4-prepare-changelist",
    "p4-post-changelist",
    "p4-pre-submit",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookError {
    UnknownHook,
    NoSuchHook,
    Symlink,
    SharedNeedsConfirmation(PathBuf),
    AlreadyInState,
    Io(String),
    Refused(String),
    Exec(ExecError),
}

impl std::fmt::Display for HookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownHook => f.write_str("ce nom n'est pas un hook connu de Git"),
            Self::NoSuchHook => f.write_str("ce hook n'existe pas dans le dossier effectif"),
            Self::Symlink => f.write_str(
                "ce hook est un lien symbolique : changer son activation modifierait la cible, souvent partagée ; fait-le à la main si c'est voulu",
            ),
            Self::SharedNeedsConfirmation(dir) => write!(
                f,
                "ce dossier de hooks ({}) est hors du dépôt : le changement vaudrait pour tous les dépôts qui l'utilisent, une confirmation explicite est requise",
                dir.display()
            ),
            Self::AlreadyInState => f.write_str("ce hook est déjà dans l'état demandé"),
            Self::Io(m) => write!(f, "erreur système : {m}"),
            Self::Refused(m) => write!(f, "Git refuse : {m}"),
            Self::Exec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for HookError {}

impl From<ExecError> for HookError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

impl Git {
    pub async fn list_hooks(&self, info: &SessionInfo) -> Result<HooksView, HookError> {
        let dir = self
            .exec_ok(
                info,
                &["rev-parse", "--path-format=absolute", "--git-path", "hooks"],
            )
            .await?
            .text();
        let dir = PathBuf::from(dir);
        let configured = self
            .exec(
                info,
                &["config", "--show-scope", "--get", "core.hooksPath"],
                &[],
                None,
            )
            .await?;
        let (configured_scope, configured_path) = if configured.ok() {
            let text = configured.text();
            match text.split_once('\t') {
                Some((scope, value)) => (Some(scope.to_owned()), Some(value.to_owned())),
                None => (None, Some(text)),
            }
        } else {
            (None, None)
        };
        let canonical = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        let repo_hooks =
            std::fs::canonicalize(&info.common_dir).unwrap_or_else(|_| info.common_dir.clone());
        let shared = !canonical.starts_with(&repo_hooks);
        let in_worktree = info
            .worktree
            .as_deref()
            .and_then(|w| std::fs::canonicalize(w).ok())
            .is_some_and(|w| canonical.starts_with(w) && !canonical.starts_with(&repo_hooks));
        let mut hooks = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".sample") {
                    continue;
                }
                let path = entry.path();
                let Ok(link_meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                let meta = std::fs::metadata(&path).ok();
                if meta.as_ref().is_some_and(|m| m.is_dir()) {
                    continue;
                }
                let interpreter = std::fs::File::open(&path).ok().and_then(|mut f| {
                    let mut head = [0u8; 120];
                    let n = f.read(&mut head).ok()?;
                    let text = String::from_utf8_lossy(&head[..n]).into_owned();
                    text.lines()
                        .next()
                        .and_then(|l| l.strip_prefix("#!"))
                        .map(|l| l.trim().to_owned())
                });
                hooks.push(HookInfo {
                    known: KNOWN_HOOKS.contains(&name.as_str()),
                    executable: meta
                        .as_ref()
                        .is_some_and(|m| m.permissions().mode() & 0o111 != 0),
                    symlink: link_meta.file_type().is_symlink(),
                    size: meta.map_or(0, |m| m.len()),
                    name,
                    path,
                    interpreter,
                });
            }
        }
        hooks.sort_by(|a, b| b.known.cmp(&a.known).then(a.name.cmp(&b.name)));
        Ok(HooksView {
            dir,
            configured_path,
            configured_scope,
            shared,
            in_worktree,
            hooks,
        })
    }

    pub async fn set_hook_enabled(
        &self,
        info: &SessionInfo,
        name: &str,
        enabled: bool,
        confirm_shared: bool,
    ) -> Result<HooksView, HookError> {
        if !KNOWN_HOOKS.contains(&name) {
            return Err(HookError::UnknownHook);
        }
        let view = self.list_hooks(info).await?;
        let hook = view
            .hooks
            .iter()
            .find(|h| h.name == name)
            .ok_or(HookError::NoSuchHook)?;
        if hook.symlink {
            return Err(HookError::Symlink);
        }
        if hook.executable == enabled {
            return Err(HookError::AlreadyInState);
        }
        if view.shared && !confirm_shared {
            return Err(HookError::SharedNeedsConfirmation(view.dir.clone()));
        }
        let meta = std::fs::metadata(&hook.path).map_err(|e| HookError::Io(e.to_string()))?;
        let mut mode = meta.permissions().mode();
        if enabled {
            mode |= (mode & 0o444) >> 2;
        } else {
            mode &= !0o111;
        }
        std::fs::set_permissions(&hook.path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| HookError::Io(e.to_string()))?;
        self.list_hooks(info).await
    }
}
