use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gitpoulpe_core::{SessionInfo, Submodule, SubmoduleState, SubmoduleUpdated, mask_url};

use crate::net::{NetError, NetProgress};
use crate::remote::validate_remote_url;
use crate::worktree::WorktreeError;
use crate::{Cancel, Git};

const MAX_DEPTH: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmoduleError {
    NoSuchSubmodule,
    UrlRefused(String),
    NotInitialized,
    Dirty(Vec<PathBuf>),
    UnsavedCommits(Vec<String>),
    Net(NetError),
    Refused(String),
}

impl std::fmt::Display for SubmoduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchSubmodule => f.write_str("ce submodule n'existe pas dans ce dépôt"),
            Self::UrlRefused(why) => write!(
                f,
                "l'URL déclarée par .gitmodules est refusée ({why}) : rien n'a été initialisé ni exécuté"
            ),
            Self::NotInitialized => f.write_str("ce submodule n'est pas initialisé"),
            Self::Dirty(files) => write!(
                f,
                "le submodule contient des modifications locales ({}) : elles sont protégées, rien n'a été mis à jour",
                files
                    .iter()
                    .take(5)
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::UnsavedCommits(commits) => {
                let (noun, effect) = if commits.len() > 1 {
                    ("commits", "les laisserait orphelins")
                } else {
                    ("commit", "le laisserait orphelin")
                };
                write!(
                    f,
                    "le submodule porte {} {noun} qu'aucune branche n'atteint ({}) : la mise à jour {effect}, une décision explicite est requise",
                    commits.len(),
                    commits
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" ; ")
                )
            }
            Self::Net(e) => write!(f, "{e} ; l'état local du submodule n'a pas changé"),
            Self::Refused(m) => write!(f, "Git refuse : {m}"),
        }
    }
}

impl std::error::Error for SubmoduleError {}

fn map(error: WorktreeError) -> SubmoduleError {
    SubmoduleError::Refused(error.to_string())
}

fn is_checked_out(dir: &Path) -> bool {
    dir.join(".git").exists()
}

impl Git {
    pub async fn list_submodules(
        &self,
        info: &SessionInfo,
    ) -> Result<Vec<Submodule>, SubmoduleError> {
        let Some(root) = info.worktree.clone() else {
            return Ok(Vec::new());
        };
        let mut found = Vec::new();
        self.scan_submodules(&root, Path::new(""), 0, &mut found)
            .await?;
        Ok(found)
    }

    async fn scan_submodules(
        &self,
        dir: &Path,
        prefix: &Path,
        depth: usize,
        out: &mut Vec<Submodule>,
    ) -> Result<(), SubmoduleError> {
        if depth > MAX_DEPTH {
            return Ok(());
        }
        let staged = self
            .in_dir(dir, &["ls-files", "-s", "-z"])
            .await
            .map_err(map)?;
        let mut links: Vec<(PathBuf, String, bool)> = Vec::new();
        for record in staged.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let text = String::from_utf8_lossy(record).into_owned();
            let Some((meta, path)) = text.split_once('\t') else {
                continue;
            };
            let mut fields = meta.split(' ');
            let (Some(mode), Some(oid), Some(stage)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if mode == "160000" {
                let conflict = stage != "0";
                if let Some(existing) = links.iter_mut().find(|(p, _, _)| p == Path::new(path)) {
                    existing.2 |= conflict;
                } else {
                    links.push((PathBuf::from(path), oid.to_owned(), conflict));
                }
            }
        }
        if links.is_empty() {
            return Ok(());
        }
        let config = self
            .in_dir(dir, &["config", "--file", ".gitmodules", "-z", "--list"])
            .await
            .map_err(map)?;
        let mut declared: BTreeMap<PathBuf, (String, Option<String>)> = BTreeMap::new();
        let mut names: BTreeMap<String, (Option<PathBuf>, Option<String>)> = BTreeMap::new();
        for entry in String::from_utf8_lossy(&config.stdout).split('\0') {
            let Some((key, value)) = entry.split_once('\n') else {
                continue;
            };
            let Some(rest) = key.strip_prefix("submodule.") else {
                continue;
            };
            if let Some(name) = rest.strip_suffix(".path") {
                names.entry(name.to_owned()).or_default().0 = Some(PathBuf::from(value));
            } else if let Some(name) = rest.strip_suffix(".url") {
                names.entry(name.to_owned()).or_default().1 = Some(value.to_owned());
            }
        }
        for (name, (path, url)) in names {
            if let Some(path) = path {
                declared.insert(path, (name, url));
            }
        }
        for (path, expected, conflict) in links {
            let (name, raw_url) = declared
                .get(&path)
                .cloned()
                .unwrap_or_else(|| (path.display().to_string(), None));
            let (url, url_refused) = match &raw_url {
                None => (None, Some("aucune URL dans .gitmodules".to_owned())),
                Some(url) => match validate_remote_url(url) {
                    Ok(()) => (Some(mask_url(url)), None),
                    Err(e) => (None, Some(e.to_string())),
                },
            };
            let initialized = self
                .in_dir(dir, &["config", "--get", &format!("submodule.{name}.url")])
                .await
                .map_err(map)?
                .ok();
            let full = dir.join(&path);
            let checked_out = is_checked_out(&full);
            let mut current = None;
            let (mut ahead, mut behind, mut expected_available) = (0, 0, false);
            let (mut modified, mut untracked) = (0, 0);
            if checked_out {
                let head = self
                    .in_dir(&full, &["rev-parse", "--verify", "-q", "HEAD"])
                    .await
                    .map_err(map)?;
                current = head.ok().then(|| head.text());
                expected_available = self
                    .in_dir(
                        &full,
                        &["cat-file", "-e", &format!("{expected}^{{commit}}")],
                    )
                    .await
                    .map_err(map)?
                    .ok();
                if let (Some(cur), true) = (&current, expected_available) {
                    let counts = self
                        .in_dir(
                            &full,
                            &[
                                "rev-list",
                                "--left-right",
                                "--count",
                                &format!("{expected}...{cur}"),
                            ],
                        )
                        .await
                        .map_err(map)?
                        .text();
                    let mut numbers = counts
                        .split_whitespace()
                        .map(|n| n.parse::<usize>().unwrap_or(0));
                    (behind, ahead) = (numbers.next().unwrap_or(0), numbers.next().unwrap_or(0));
                }
                let status = self
                    .in_dir(
                        &full,
                        &["status", "--porcelain", "-z", "--untracked-files=normal"],
                    )
                    .await
                    .map_err(map)?;
                for record in status.stdout.split(|b| *b == 0).filter(|r| r.len() > 2) {
                    if record.starts_with(b"??") {
                        untracked += 1;
                    } else {
                        modified += 1;
                    }
                }
            }
            let state = if conflict {
                SubmoduleState::Conflict
            } else if !initialized {
                SubmoduleState::NotInitialized
            } else if !checked_out {
                SubmoduleState::Initialized
            } else if current.as_deref() == Some(expected.as_str()) {
                SubmoduleState::UpToDate
            } else {
                SubmoduleState::Differs
            };
            let shown = prefix.join(&path);
            out.push(Submodule {
                name,
                path: shown.clone(),
                url,
                url_refused,
                expected,
                current,
                state,
                ahead,
                behind,
                expected_available,
                modified,
                untracked,
                depth,
            });
            if checked_out {
                Box::pin(self.scan_submodules(&full, &shown, depth + 1, out)).await?;
            }
        }
        Ok(())
    }

    async fn locate(
        &self,
        info: &SessionInfo,
        path: &Path,
    ) -> Result<(PathBuf, PathBuf, Submodule), SubmoduleError> {
        let all = self.list_submodules(info).await?;
        let target = all
            .iter()
            .find(|s| s.path == path)
            .cloned()
            .ok_or(SubmoduleError::NoSuchSubmodule)?;
        let root = info
            .worktree
            .clone()
            .ok_or(SubmoduleError::NoSuchSubmodule)?;
        let owner = all
            .iter()
            .filter(|s| {
                path.starts_with(&s.path) && s.path != path && is_checked_out(&root.join(&s.path))
            })
            .max_by_key(|s| s.path.components().count());
        Ok(match owner {
            Some(parent) => (
                root.join(&parent.path),
                path.strip_prefix(&parent.path)
                    .unwrap_or(path)
                    .to_path_buf(),
                target,
            ),
            None => (root, path.to_path_buf(), target),
        })
    }

    pub async fn init_submodule(
        &self,
        info: &SessionInfo,
        path: &Path,
    ) -> Result<(), SubmoduleError> {
        let (dir, relative, target) = self.locate(info, path).await?;
        if let Some(why) = target.url_refused {
            return Err(SubmoduleError::UrlRefused(why));
        }
        let out = self
            .in_dir(
                &dir,
                &["submodule", "init", "--", &relative.to_string_lossy()],
            )
            .await
            .map_err(map)?;
        if out.ok() {
            Ok(())
        } else {
            Err(SubmoduleError::Refused(out.stderr))
        }
    }

    pub async fn update_submodule(
        &self,
        info: &SessionInfo,
        path: &Path,
        confirm_unsaved_commits: bool,
        cancel: &Cancel,
        on_progress: impl FnMut(NetProgress),
    ) -> Result<SubmoduleUpdated, SubmoduleError> {
        let (dir, relative, target) = self.locate(info, path).await?;
        if let Some(why) = &target.url_refused {
            return Err(SubmoduleError::UrlRefused(why.clone()));
        }
        if target.state == SubmoduleState::NotInitialized {
            self.init_submodule(info, path).await?;
        }
        let full = dir.join(&relative);
        let mut marker = None;
        if is_checked_out(&full) {
            let status = self
                .in_dir(
                    &full,
                    &["status", "--porcelain", "-z", "--untracked-files=no"],
                )
                .await
                .map_err(map)?;
            let dirty: Vec<PathBuf> = status
                .stdout
                .split(|b| *b == 0)
                .filter(|r| r.len() > 3)
                .map(|r| PathBuf::from(String::from_utf8_lossy(&r[3..]).into_owned()))
                .collect();
            if !dirty.is_empty() {
                return Err(SubmoduleError::Dirty(dirty));
            }
            if let Some(current) = &target.current
                && current != &target.expected
            {
                let reachable = self
                    .in_dir(
                        &full,
                        &[
                            "for-each-ref",
                            "--contains",
                            current,
                            "--format=%(refname)",
                            "refs/heads",
                            "refs/remotes",
                            "refs/tags",
                        ],
                    )
                    .await
                    .map_err(map)?;
                if reachable.text().is_empty() {
                    let log = self
                        .in_dir(
                            &full,
                            &[
                                "log",
                                "--format=%h %s",
                                "--max-count=20",
                                current,
                                "--not",
                                "--branches",
                                "--remotes",
                                "--tags",
                                "--",
                            ],
                        )
                        .await
                        .map_err(map)?;
                    let commits: Vec<String> = log.text().lines().map(str::to_owned).collect();
                    if !confirm_unsaved_commits {
                        return Err(SubmoduleError::UnsavedCommits(commits));
                    }
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs());
                    let name = format!("refs/gitpoulpe/recovery/{stamp}-submodule");
                    let made = self
                        .in_dir(&full, &["update-ref", &name, current])
                        .await
                        .map_err(map)?;
                    if !made.ok() {
                        return Err(SubmoduleError::Refused(made.stderr));
                    }
                    marker = Some(name);
                }
            }
        }
        let run = self
            .run_network(
                &dir,
                &["submodule", "update", "--", &relative.to_string_lossy()],
                cancel,
                on_progress,
            )
            .await
            .map_err(SubmoduleError::Net)?;
        if run.code != Some(0) {
            let url = target.url.clone();
            return Err(
                match self.classify_net_failure(&run.stderr, url.as_deref()).await {
                    NetError::Refused(m) => SubmoduleError::Refused(m),
                    other => SubmoduleError::Net(other),
                },
            );
        }
        let head = self
            .in_dir(&full, &["rev-parse", "HEAD"])
            .await
            .map_err(map)?
            .text();
        if head != target.expected {
            return Err(SubmoduleError::Refused(
                "le submodule n'est pas sur le commit attendu après la mise à jour".to_owned(),
            ));
        }
        Ok(SubmoduleUpdated {
            path: path.to_path_buf(),
            from: target.current,
            to: head,
            marker,
        })
    }
}
