use std::path::{Path, PathBuf};

use gitpoulpe_core::{DeepLink, Favorite, LINK_SCHEME, LinkTarget, parse_link};
use gitpoulpe_git::commit_exists;
use rune::TextEdit;

use crate::engine::Command;
use crate::notices::Target;
use crate::state::State;

#[derive(Default)]
pub struct LinkState {
    pub open: bool,
    pub input: TextEdit,
    pub link: Option<DeepLink>,
    pub matches: Option<Vec<String>>,
    pub busy: bool,
    pub error: Option<String>,
    pub request: u64,
    pub version: u32,
}

pub fn link_from_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> Option<String> {
    args.into_iter()
        .skip(1)
        .filter_map(|a| a.into_string().ok())
        .find(|a| a.starts_with(LINK_SCHEME))
}

impl State {
    pub fn open_deep_link(&mut self, input: Option<String>) {
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
        self.close_overview();
        self.close_notices();
        self.close_issues();
        self.close_patch_import();
        self.accounts.open = false;
        self.profiles_open = false;
        let request = self.links.request + 1;
        self.links = LinkState {
            open: true,
            request,
            input: TextEdit::new(input.clone().unwrap_or_default()),
            ..LinkState::default()
        };
        if input.is_some() {
            self.link_analyse();
        }
    }

    pub fn close_deep_link(&mut self) {
        self.links.open = false;
        self.links.busy = false;
        self.links.request += 1;
    }

    fn link_failure(&mut self, message: String) {
        self.links.error = Some(message);
        self.links.version += 1;
    }

    fn known_repositories(&self) -> Vec<String> {
        let mut paths: Vec<String> = Vec::new();
        if let Some(repo) = &self.repo {
            paths.push(repo.path.display().to_string());
        }
        for workspace in &self.workspaces.book.items {
            paths.extend(workspace.repos.iter().map(|r| r.path.clone()));
        }
        for favorite in &self.favorites.book.items {
            if let Favorite::Local { path, .. } = favorite {
                paths.push(path.clone());
            }
        }
        let mut seen = std::collections::HashSet::new();
        paths.retain(|p| seen.insert(PathBuf::from(p).canonicalize().unwrap_or_else(|_| p.into())));
        paths
    }

    pub fn link_analyse(&mut self) {
        self.links.error = None;
        self.links.link = None;
        self.links.matches = None;
        match parse_link(&self.links.input.text) {
            Err(error) => self.link_failure(error.to_string()),
            Ok(link) => {
                self.links.request += 1;
                self.links.busy = true;
                self.links.link = Some(link.clone());
                self.links.version += 1;
                let request = self.links.request;
                let paths = self.known_repositories();
                if let Some(engine) = &self.engine {
                    engine.send(Command::LinkResolve {
                        request,
                        link,
                        paths,
                    });
                }
            }
        }
    }

    pub fn apply_link_resolved(&mut self, request: u64, matches: Vec<String>) -> bool {
        if !self.links.open || request != self.links.request {
            return false;
        }
        self.links.busy = false;
        self.links.matches = Some(matches);
        self.links.version += 1;
        true
    }

    pub fn link_go(&mut self, index: usize) {
        let Some(link) = self.links.link.clone() else {
            return;
        };
        let Some(path) = self
            .links
            .matches
            .as_ref()
            .and_then(|m| m.get(index))
            .cloned()
        else {
            return;
        };
        let target = match &link.target {
            LinkTarget::Repository => None,
            LinkTarget::Commit(sha) => {
                if !commit_exists(Path::new(&path), sha) {
                    self.link_failure(format!(
                        "Le commit {} n'est pas dans ce dépôt local : récupérez les références distantes (Dépôts distants) puis rouvrez le lien.",
                        sha.chars().take(10).collect::<String>()
                    ));
                    return;
                }
                Some(Target::Commit(sha.clone()))
            }
            LinkTarget::Pull(number) => Some(Target::Pull {
                number: Some(*number),
            }),
        };
        self.close_deep_link();
        match target {
            Some(target) => self.notices_go(path, target),
            None => {
                let same = self.repo.as_ref().is_some_and(|r| {
                    r.path.canonicalize().ok() == PathBuf::from(&path).canonicalize().ok()
                });
                if !same {
                    self.open_path(PathBuf::from(path));
                }
            }
        }
    }
}
