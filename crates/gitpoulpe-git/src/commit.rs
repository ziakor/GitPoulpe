use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use gitpoulpe_core::{
    CommitFailureKind, CommitOutcome, CommitRefusal, CommitRequest, SessionInfo, SigningPreference,
    WriteError,
};

use crate::diagnostics::filter_diagnostic;
use crate::identity::{IdentityError, SigningCheck, check_signing, profile_env, signing_config};
use crate::stage::{locked_index, write_failure};
use crate::{Cancel, Git};

const HOOKS_THAT_CAN_REFUSE: [&str; 3] = ["pre-commit", "commit-msg", "prepare-commit-msg"];
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

fn signing_failure(output: &str) -> bool {
    let lower = output.to_lowercase();
    [
        "failed to sign",
        "gpg failed",
        "signing failed",
        "load key",
        "ssh-keygen",
        "couldn't load public key",
        "no secret key",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

impl Git {
    async fn collect(&self, info: &SessionInfo, args: &[&str]) -> Option<String> {
        let dir = info.worktree.as_deref().unwrap_or(&info.git_dir);
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let (run, out) = self.run_collect(dir, &refs, &Cancel::new()).await.ok()?;
        run.succeeded()
            .then(|| String::from_utf8_lossy(&out).trim().to_owned())
    }

    async fn head(&self, info: &SessionInfo) -> Option<String> {
        self.collect(info, &["rev-parse", "--verify", "-q", "HEAD"])
            .await
            .filter(|oid| !oid.is_empty())
    }

    async fn refusing_hook_present(&self, info: &SessionInfo) -> bool {
        for name in HOOKS_THAT_CAN_REFUSE {
            let arg = format!("hooks/{name}");
            let Some(path) = self.collect(info, &["rev-parse", "--git-path", &arg]).await else {
                continue;
            };
            let dir = info.worktree.as_deref().unwrap_or(&info.git_dir);
            let path = dir.join(PathBuf::from(path));
            if std::fs::metadata(&path)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            {
                return true;
            }
        }
        false
    }

    pub async fn commit(
        &self,
        info: &SessionInfo,
        request: &CommitRequest,
        cancel: &Cancel,
    ) -> Result<CommitOutcome, WriteError> {
        let worktree = info.worktree.as_ref().ok_or(WriteError::NoWorktree)?;
        let message = request.message.trim_end();
        if message.trim().is_empty() {
            return Ok(CommitOutcome::Refused(CommitRefusal::EmptyMessage));
        }
        let current = self.current_status(info, cancel).await?;
        if current.entries.iter().any(|e| e.is_conflict()) {
            return Ok(CommitOutcome::Refused(CommitRefusal::Conflicts));
        }
        let now: BTreeSet<(PathBuf, String)> = current
            .entries
            .iter()
            .filter(|e| e.is_staged())
            .map(|e| {
                (
                    e.path.clone(),
                    current.index_ids.get(&e.path).cloned().unwrap_or_default(),
                )
            })
            .collect();
        if now.is_empty() {
            return Ok(CommitOutcome::Refused(CommitRefusal::EmptyIndex));
        }
        let seen: BTreeSet<(PathBuf, String)> = request.staged.iter().cloned().collect();
        if seen != now {
            return Ok(CommitOutcome::Refused(CommitRefusal::Stale));
        }
        let profile = request.profile.as_ref();
        match self.effective_identity(info, profile, cancel).await {
            Ok(_) => {}
            Err(IdentityError::Unknown(message)) => {
                return Ok(CommitOutcome::Failed {
                    kind: CommitFailureKind::IdentityUnknown,
                    output: message,
                });
            }
            Err(IdentityError::Cancelled) => return Err(WriteError::Cancelled),
            Err(IdentityError::GitUnavailable(d)) => return Err(WriteError::GitUnavailable(d)),
            Err(IdentityError::GitFailed(d)) => return Err(WriteError::GitFailed(d)),
        }
        if let Some(profile) = profile
            && profile.signing != SigningPreference::None
            && let SigningCheck::Problem(message) = check_signing(profile).await
        {
            return Ok(CommitOutcome::Failed {
                kind: CommitFailureKind::SigningUnavailable,
                output: message,
            });
        }

        let before = self.head(info).await;
        let mut args: Vec<OsString> = Vec::new();
        if let Some(profile) = profile {
            for (key, value) in signing_config(profile) {
                args.push("-c".into());
                args.push(format!("{key}={value}").into());
            }
        }
        args.extend(["commit", "--cleanup=whitespace", "-F", "-"].map(OsString::from));
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let mut env = profile.map(profile_env).unwrap_or_default();
        env.push((OsString::from("GIT_EDITOR"), OsString::from(":")));
        let mut stdout = Vec::new();
        let run = self
            .run_input(
                worktree,
                &refs,
                &env,
                Some(format!("{message}\n").into_bytes()),
                cancel,
                |chunk| {
                    let room = MAX_OUTPUT_BYTES.saturating_sub(stdout.len());
                    stdout.extend_from_slice(&chunk[..chunk.len().min(room)]);
                    Ok(())
                },
            )
            .await
            .map_err(write_failure)?;
        if !run.succeeded() {
            let mut output = filter_diagnostic(&run.stderr);
            let printed = filter_diagnostic(&stdout);
            if !printed.is_empty() {
                if !output.is_empty() {
                    output.push('\n');
                }
                output.push_str(&printed);
            }
            let diagnostic = gitpoulpe_core::Diagnostic {
                exit_code: run.code,
                signal: run.signal,
                message: output.clone(),
                cause: None,
            };
            if let Some(path) = locked_index(&diagnostic) {
                return Err(WriteError::IndexLocked(path));
            }
            let kind = if signing_failure(&output) {
                CommitFailureKind::SigningUnavailable
            } else if self.refusing_hook_present(info).await {
                CommitFailureKind::Hook
            } else {
                CommitFailureKind::Other
            };
            return Ok(CommitOutcome::Failed { kind, output });
        }
        let oid = self.head(info).await.unwrap_or_default();
        let signed = self
            .collect(info, &["cat-file", "commit", "HEAD"])
            .await
            .is_some_and(|text| text.lines().any(|l| l.starts_with("gpgsig")));
        Ok(CommitOutcome::Created {
            subject: message.lines().next().unwrap_or_default().to_owned(),
            oid,
            before,
            signed,
        })
    }
}
