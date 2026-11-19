use std::collections::{BTreeMap, BTreeSet};

use gitpoulpe_core::Favorite;
use gitpoulpe_git::RepoOverview;
use std::path::{Path, PathBuf};

use crate::engine::Command;
use crate::state::State;
use crate::workspaces::{RepoHealth, health};

pub const OVERVIEW_LIMIT: usize = 4;
pub const OVERVIEW_TIMEOUT_MS: u64 = 30_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowState {
    Loading,
    Ready(RepoOverview),
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Row {
    pub job: u64,
    pub state: RowState,
}

pub struct Candidate {
    pub path: String,
    pub name: String,
    pub sources: Vec<String>,
}

pub struct OverviewState {
    pub open: bool,
    pub selected: BTreeSet<String>,
    pub known: BTreeSet<String>,
    pub rows: BTreeMap<String, Row>,
    pub next_job: u64,
    pub timeout_ms: u64,
    pub notice: Option<String>,
    pub version: u32,
}

impl Default for OverviewState {
    fn default() -> Self {
        Self {
            open: false,
            selected: BTreeSet::new(),
            known: BTreeSet::new(),
            rows: BTreeMap::new(),
            next_job: 0,
            timeout_ms: OVERVIEW_TIMEOUT_MS,
            notice: None,
            version: 0,
        }
    }
}

fn canonical(path: &str) -> String {
    Path::new(path)
        .canonicalize()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_else(|| path.to_owned())
}

fn name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

impl State {
    pub fn overview_candidates(&self) -> Vec<Candidate> {
        let mut found: BTreeMap<String, Candidate> = BTreeMap::new();
        let mut add = |path: &str, source: String| {
            let key = canonical(path);
            found
                .entry(key.clone())
                .or_insert_with(|| Candidate {
                    path: key.clone(),
                    name: name_of(&key),
                    sources: Vec::new(),
                })
                .sources
                .push(source);
        };
        if let Some(repo) = self.repo.as_ref().and_then(|r| r.path.to_str()) {
            add(repo, "dépôt ouvert".to_owned());
        }
        for workspace in &self.workspaces.book.items {
            for repo in &workspace.repos {
                add(&repo.path, format!("espace « {} »", workspace.name));
            }
        }
        for favorite in &self.favorites.book.items {
            if let Favorite::Local { path, .. } = favorite {
                add(path, "favori".to_owned());
            }
        }
        found.into_values().collect()
    }

    pub fn open_overview(&mut self) {
        self.close_history();
        self.close_ignored();
        self.close_branches();
        self.close_tags();
        self.close_remotes();
        self.close_conflicts();
        self.close_stashes();
        self.close_reflog();
        self.close_undo();
        self.close_worktrees();
        self.close_hooks();
        self.close_lfs();
        self.close_submodules();
        self.close_gitflow();
        self.close_pulls();
        self.close_file_history();
        self.close_file_edit();
        self.close_tools();
        self.close_terminal();
        self.close_shortcuts();
        self.close_workspaces();
        self.close_favorites();
        self.close_notices();
        self.close_issues();
        self.accounts.open = false;
        self.profiles_open = false;
        let candidates: BTreeSet<String> = self
            .overview_candidates()
            .into_iter()
            .map(|c| c.path)
            .collect();
        let fresh: Vec<String> = candidates
            .difference(&self.overview.known)
            .cloned()
            .collect();
        self.overview.selected.retain(|p| candidates.contains(p));
        self.overview.selected.extend(fresh);
        self.overview.known = candidates;
        self.overview
            .rows
            .retain(|p, _| self.overview.selected.contains(p));
        self.overview.open = true;
        self.overview.notice = None;
        self.overview.version += 1;
        self.overview_refresh_all();
    }

    pub fn close_overview(&mut self) {
        self.overview.open = false;
    }

    fn start_overview_jobs(&mut self, paths: Vec<String>) {
        if paths.is_empty() {
            return;
        }
        let mut jobs = Vec::new();
        for path in paths {
            self.overview.next_job += 1;
            let job = self.overview.next_job;
            self.overview.rows.insert(
                path.clone(),
                Row {
                    job,
                    state: RowState::Loading,
                },
            );
            jobs.push((path, job));
        }
        self.overview.version += 1;
        let timeout_ms = self.overview.timeout_ms;
        if let Some(engine) = &self.engine {
            engine.send(Command::OverviewRefresh { jobs, timeout_ms });
        }
    }

    pub fn overview_refresh_all(&mut self) {
        let paths: Vec<String> = self.overview.selected.iter().cloned().collect();
        self.start_overview_jobs(paths);
    }

    pub fn overview_refresh_one(&mut self, path: &str) {
        if self.overview.selected.contains(path) {
            self.start_overview_jobs(vec![path.to_owned()]);
        }
    }

    pub fn overview_toggle(&mut self, path: &str) {
        if self.overview.selected.remove(path) {
            self.overview.rows.remove(path);
            self.overview.version += 1;
            if let Some(engine) = &self.engine {
                engine.send(Command::OverviewCancel {
                    paths: vec![path.to_owned()],
                });
            }
        } else {
            self.overview.selected.insert(path.to_owned());
            self.start_overview_jobs(vec![path.to_owned()]);
        }
    }

    pub fn overview_select_all(&mut self) {
        let all: Vec<String> = self
            .overview_candidates()
            .into_iter()
            .map(|c| c.path)
            .collect();
        let added: Vec<String> = all
            .iter()
            .filter(|p| !self.overview.selected.contains(*p))
            .cloned()
            .collect();
        self.overview.selected.extend(all);
        self.start_overview_jobs(added);
    }

    pub fn overview_select_none(&mut self) {
        let paths: Vec<String> = self.overview.selected.iter().cloned().collect();
        self.overview.selected.clear();
        self.overview.rows.clear();
        self.overview.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::OverviewCancel { paths });
        }
    }

    pub fn apply_overview_repo(
        &mut self,
        path: String,
        job: u64,
        result: Result<RepoOverview, String>,
    ) -> bool {
        let Some(row) = self.overview.rows.get_mut(&path) else {
            return false;
        };
        if row.job != job || !self.overview.selected.contains(&path) {
            return false;
        }
        row.state = match result {
            Ok(overview) => RowState::Ready(overview),
            Err(message) => RowState::Failed(message),
        };
        self.overview.version += 1;
        true
    }

    pub fn overview_open(&mut self, path: &str) {
        if health(path) != RepoHealth::Ok {
            self.overview.notice = Some(format!(
                "{path} est introuvable ou n'est plus un dépôt : corrigez-le dans les espaces de travail ou les favoris."
            ));
            self.overview.version += 1;
            return;
        }
        let owner = self
            .workspaces
            .book
            .items
            .iter()
            .filter(|w| w.repos.iter().any(|r| canonical(&r.path) == path))
            .map(|w| (self.workspaces.book.active == Some(w.id), w))
            .max_by_key(|(active, _)| *active)
            .map(|(_, w)| {
                (
                    w.id,
                    w.repos
                        .iter()
                        .find(|r| canonical(&r.path) == path)
                        .map(|r| r.path.clone()),
                )
            });
        self.close_overview();
        match owner {
            Some((id, Some(stored))) => {
                self.workspaces.selected = Some(id);
                self.workspace_open_repo(stored);
            }
            _ => self.open_path(PathBuf::from(path)),
        }
    }

    pub fn overview_saved_position(&self, path: &str) -> Option<(String, Option<String>)> {
        self.workspaces
            .book
            .items
            .iter()
            .flat_map(|w| w.repos.iter())
            .find(|r| canonical(&r.path) == path)
            .map(|r| (r.view.clone(), r.file.clone()))
    }
}
