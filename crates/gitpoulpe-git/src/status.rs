use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStringExt;
use std::path::Path;

use gitpoulpe_core::{Diagnostic, Generation, ReadError, SessionInfo, StatusSnapshot};

use crate::parse::StatusParser;
use crate::process::RunError;
use crate::{Cancel, Git};

fn read_failure(error: RunError) -> ReadError {
    match error {
        RunError::Cancelled => ReadError::Cancelled,
        RunError::Spawn(e) => ReadError::GitUnavailable(RunError::spawn_diagnostic(&e)),
        RunError::Io(e) => ReadError::GitFailed(RunError::spawn_diagnostic(&e)),
        RunError::Rejected(message) => ReadError::Parse(message),
    }
}

fn filter_drivers(config_names: &[u8]) -> Result<BTreeSet<Vec<u8>>, ReadError> {
    let mut drivers = BTreeSet::new();
    for entry in config_names.split(|&b| b == 0).filter(|e| !e.is_empty()) {
        let driver = entry
            .strip_prefix(b"filter.")
            .and_then(|rest| {
                rest.iter()
                    .rposition(|&b| b == b'.')
                    .map(|dot| &rest[..dot])
            })
            .filter(|driver| !driver.is_empty());
        match driver {
            Some(driver) => {
                drivers.insert(driver.to_vec());
            }
            None => {
                return Err(ReadError::Parse(
                    "unexpected filter configuration entry".to_owned(),
                ));
            }
        }
    }
    Ok(drivers)
}

fn neutralise_filters(drivers: &BTreeSet<Vec<u8>>) -> Vec<(OsString, OsString)> {
    let mut settings = Vec::new();
    for driver in drivers {
        for (key, value) in [
            ("clean", ""),
            ("smudge", ""),
            ("process", ""),
            ("required", "false"),
        ] {
            let mut name = b"filter.".to_vec();
            name.extend_from_slice(driver);
            name.extend_from_slice(format!(".{key}").as_bytes());
            settings.push((OsString::from_vec(name), OsString::from(value)));
        }
    }
    let mut env = vec![(
        OsString::from("GIT_CONFIG_COUNT"),
        OsString::from(settings.len().to_string()),
    )];
    for (index, (name, value)) in settings.into_iter().enumerate() {
        env.push((OsString::from(format!("GIT_CONFIG_KEY_{index}")), name));
        env.push((OsString::from(format!("GIT_CONFIG_VALUE_{index}")), value));
    }
    env
}

impl Git {
    pub(crate) async fn filter_overrides(
        &self,
        dir: &Path,
        cancel: &Cancel,
    ) -> Result<Vec<(OsString, OsString)>, ReadError> {
        let args =
            ["config", "-z", "--name-only", "--get-regexp", "^filter\\."].map(OsString::from);
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let (run, output) = self
            .run_collect(dir, &refs, cancel)
            .await
            .map_err(read_failure)?;
        match run.code {
            Some(0) => {
                let drivers = filter_drivers(&output)?;
                Ok(if drivers.is_empty() {
                    Vec::new()
                } else {
                    neutralise_filters(&drivers)
                })
            }
            Some(1) => Ok(Vec::new()),
            _ => Err(ReadError::GitFailed(run.diagnostic())),
        }
    }

    pub async fn read_status(
        &self,
        info: &SessionInfo,
        generation: Generation,
        cancel: &Cancel,
    ) -> Result<StatusSnapshot, ReadError> {
        let worktree = info.worktree.as_ref().ok_or(ReadError::NoWorktree)?;
        let env = self.filter_overrides(worktree, cancel).await?;
        let args = [
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=all",
            "--no-ahead-behind",
            "--ignore-submodules=all",
        ]
        .map(OsString::from);
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let mut parser = StatusParser::new();
        let run = self
            .run_env(worktree, &refs, &env, cancel, |chunk| {
                parser.push(chunk).map_err(|e| e.to_string())
            })
            .await
            .map_err(read_failure)?;
        if !run.succeeded() {
            return Err(ReadError::GitFailed(Diagnostic {
                cause: None,
                ..run.diagnostic()
            }));
        }
        let mut snapshot = parser
            .finish(info.repo.clone(), info.id, generation)
            .map_err(|e| ReadError::Parse(e.to_string()))?;
        for entry in snapshot.entries.iter().filter(|e| e.is_unstaged()) {
            snapshot.stamps.insert(
                entry.path.clone(),
                crate::stage::stamp_of(&worktree.join(&entry.path)),
            );
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    #[test]
    fn drivers_are_extracted_from_nul_separated_names_including_dotted_and_equals_ones() {
        let names =
            b"filter.lfs.clean\0filter.lfs.smudge\0filter.my.driver.process\0filter.a=b.clean\0";
        let drivers = filter_drivers(names).unwrap();
        let expected: BTreeSet<Vec<u8>> =
            [b"lfs".to_vec(), b"my.driver".to_vec(), b"a=b".to_vec()].into();
        assert_eq!(drivers, expected);
        let env = neutralise_filters(&drivers);
        assert!(
            env.iter()
                .any(|(k, v)| k == "GIT_CONFIG_KEY_0" && v.as_bytes().starts_with(b"filter."))
        );
        assert!(
            env.iter()
                .any(|(k, v)| k == "GIT_CONFIG_COUNT" && v == "12")
        );
    }

    #[test]
    fn an_unexpected_entry_fails_closed() {
        assert!(filter_drivers(b"filter.ok.clean\0core.unrelated\0").is_err());
        assert!(filter_drivers(b"filter.noseparator\0").is_err());
    }
}
