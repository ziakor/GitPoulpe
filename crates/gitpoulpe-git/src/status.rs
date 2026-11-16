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

fn filter_drivers(config_names: &[u8]) -> BTreeSet<Vec<u8>> {
    config_names
        .split(|&b| b == b'\n')
        .filter_map(|line| line.strip_prefix(b"filter."))
        .filter_map(|rest| {
            rest.iter()
                .rposition(|&b| b == b'.')
                .map(|dot| rest[..dot].to_vec())
        })
        .filter(|driver| !driver.is_empty())
        .collect()
}

fn neutralise_filters(drivers: &BTreeSet<Vec<u8>>) -> Vec<OsString> {
    let mut args = Vec::new();
    for driver in drivers {
        for (key, value) in [
            ("clean", ""),
            ("smudge", ""),
            ("process", ""),
            ("required", "false"),
        ] {
            let mut setting = b"filter.".to_vec();
            setting.extend_from_slice(driver);
            setting.extend_from_slice(format!(".{key}={value}").as_bytes());
            args.push(OsString::from("-c"));
            args.push(OsString::from_vec(setting));
        }
    }
    args
}

impl Git {
    async fn filter_overrides(
        &self,
        dir: &Path,
        cancel: &Cancel,
    ) -> Result<Vec<OsString>, ReadError> {
        let args = ["config", "--name-only", "--get-regexp", "^filter\\."].map(OsString::from);
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let (run, output) = self
            .run_collect(dir, &refs, cancel)
            .await
            .map_err(read_failure)?;
        match run.code {
            Some(0) => Ok(neutralise_filters(&filter_drivers(&output))),
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
        let mut args = self.filter_overrides(worktree, cancel).await?;
        args.extend(
            [
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
                "--untracked-files=all",
                "--no-ahead-behind",
                "--ignore-submodules=all",
            ]
            .map(OsString::from),
        );
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let mut parser = StatusParser::new();
        let run = self
            .run(worktree, &refs, cancel, |chunk| {
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
        parser
            .finish(info.repo.clone(), info.id, generation)
            .map_err(|e| ReadError::Parse(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    #[test]
    fn drivers_are_extracted_from_config_names_including_dotted_ones() {
        let names =
            b"filter.lfs.clean\nfilter.lfs.smudge\nfilter.my.driver.process\nfilter.x.required\n";
        let drivers = filter_drivers(names);
        let expected: BTreeSet<Vec<u8>> =
            [b"lfs".to_vec(), b"my.driver".to_vec(), b"x".to_vec()].into();
        assert_eq!(drivers, expected);
        let args = neutralise_filters(&drivers);
        assert!(
            args.iter()
                .any(|a| a.as_bytes() == b"filter.my.driver.process=")
        );
        assert!(
            args.iter()
                .any(|a| a.as_bytes() == b"filter.lfs.required=false")
        );
    }
}
