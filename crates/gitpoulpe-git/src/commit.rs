use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use gitpoulpe_core::{
    AmendInfo, AmendRequest, CommitFailureKind, CommitOutcome, CommitRefusal, CommitRequest,
    SessionInfo, Signature, SignatureKind, SigningPreference, WriteError,
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
        "no private key found",
        "agent refused",
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

    async fn read_signature(
        &self,
        info: &SessionInfo,
        profile: Option<&gitpoulpe_core::Profile>,
    ) -> Option<Signature> {
        let raw = self.collect(info, &["cat-file", "commit", "HEAD"]).await?;
        let header: Vec<&str> = raw
            .lines()
            .skip_while(|line| !line.starts_with("gpgsig"))
            .take(2)
            .collect();
        let first = header.first()?;
        let kind = if first.contains("BEGIN SSH SIGNATURE")
            || header
                .get(1)
                .is_some_and(|l| l.contains("BEGIN SSH SIGNATURE"))
        {
            SignatureKind::Ssh
        } else {
            SignatureKind::Gpg
        };
        let verified = match (kind, profile.map(|p| &p.signing)) {
            (SignatureKind::Ssh, Some(SigningPreference::Ssh { public_key })) => {
                let email = profile.map(|p| p.user_email.as_str()).unwrap_or_default();
                self.verify_ssh_signature(info, public_key, email).await
            }
            _ => false,
        };
        Some(Signature { kind, verified })
    }

    pub async fn verify_ssh_signature(
        &self,
        info: &SessionInfo,
        public_key: &std::path::Path,
        email: &str,
    ) -> bool {
        let Ok(text) = std::fs::read_to_string(public_key) else {
            return false;
        };
        let mut fields = text.split_whitespace();
        let (Some(kind), Some(key)) = (fields.next(), fields.next()) else {
            return false;
        };
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir =
            std::env::temp_dir().join(format!("gitpoulpe-verify-{}-{nanos}", std::process::id()));
        if std::fs::create_dir(&dir).is_err() {
            return false;
        }
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        let allowed = dir.join("allowed_signers");
        let written = std::fs::write(&allowed, format!("{email} {kind} {key}\n")).is_ok();
        let verified = written
            && self
                .collect(
                    info,
                    &[
                        "-c",
                        &format!("gpg.ssh.allowedSignersFile={}", allowed.display()),
                        "verify-commit",
                        "HEAD",
                    ],
                )
                .await
                .is_some();
        let _ = std::fs::remove_dir_all(&dir);
        verified
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
        self.run_commit(info, request, false, cancel).await
    }

    pub async fn amend_info(&self, info: &SessionInfo) -> Option<AmendInfo> {
        let head = self.head(info).await?;
        let message = self
            .collect(info, &["log", "-1", "--format=%B"])
            .await
            .unwrap_or_default();
        let published = self
            .collect(
                info,
                &[
                    "for-each-ref",
                    "--contains",
                    "HEAD",
                    "refs/remotes",
                    "--format=%(refname)",
                ],
            )
            .await
            .unwrap_or_default();
        Some(AmendInfo {
            head,
            message,
            published_on: published
                .lines()
                .filter(|name| !name.is_empty() && !name.ends_with("/HEAD"))
                .map(|name| {
                    name.strip_prefix("refs/remotes/")
                        .unwrap_or(name)
                        .to_owned()
                })
                .collect(),
        })
    }

    pub async fn amend(
        &self,
        info: &SessionInfo,
        request: &AmendRequest,
        cancel: &Cancel,
    ) -> Result<CommitOutcome, WriteError> {
        if request.commit.message.trim().is_empty() {
            return Ok(CommitOutcome::Refused(CommitRefusal::EmptyMessage));
        }
        let Some(current) = self.amend_info(info).await else {
            return Ok(CommitOutcome::Refused(CommitRefusal::NoCommit));
        };
        if current.head != request.head {
            return Ok(CommitOutcome::Refused(CommitRefusal::Stale));
        }
        if !current.published_on.is_empty() && !request.confirmed_published {
            return Ok(CommitOutcome::Refused(CommitRefusal::Published));
        }
        let nothing_staged = self
            .collect(info, &["diff", "--cached", "--quiet"])
            .await
            .is_some();
        if nothing_staged && request.commit.message.trim_end() == current.message.trim_end() {
            return Ok(CommitOutcome::Refused(CommitRefusal::NothingToAmend));
        }
        self.run_commit(info, &request.commit, true, cancel).await
    }

    async fn run_commit(
        &self,
        info: &SessionInfo,
        request: &CommitRequest,
        amend: bool,
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
        if !amend && now.is_empty() {
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
        args.push("commit".into());
        if amend {
            args.push("--amend".into());
        }
        args.extend(["--cleanup=whitespace", "-F", "-"].map(OsString::from));
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
        let signature = self.read_signature(info, profile).await;
        Ok(CommitOutcome::Created {
            subject: message.lines().next().unwrap_or_default().to_owned(),
            oid,
            before,
            signature,
        })
    }
}
