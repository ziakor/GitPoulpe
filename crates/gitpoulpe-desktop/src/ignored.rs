use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use gitpoulpe_core::{IgnoreReason, IgnoreScope, IgnoreSource, rule_for_path};

use crate::engine::{Command, IgnoreEdit};
use crate::state::State;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReasonState {
    Loading,
    Known(Option<IgnoreReason>),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IgnoreTarget {
    pub path: PathBuf,
    pub is_dir: bool,
    pub reason: ReasonState,
}

#[derive(Default)]
pub struct IgnoredState {
    pub open: bool,
    pub request: u64,
    pub paths: Vec<PathBuf>,
    pub truncated: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub target: Option<IgnoreTarget>,
    pub version: u32,
    pub reason_request: u64,
    pub edit_request: u64,
}

pub fn scope_caption(scope: &IgnoreScope) -> String {
    match scope {
        IgnoreScope::Directory(dir) if dir.as_os_str().is_empty() => {
            ".gitignore à la racine".to_owned()
        }
        IgnoreScope::Directory(dir) => format!(".gitignore de {}", dir.display()),
        IgnoreScope::Exclude => ".git/info/exclude (local, non partagé)".to_owned(),
        IgnoreScope::Global => "exclusion globale de l'utilisateur".to_owned(),
    }
}

impl IgnoredState {
    pub fn scopes_for(path: &Path) -> Vec<IgnoreScope> {
        let mut scopes = vec![IgnoreScope::Directory(PathBuf::new())];
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            scopes.push(IgnoreScope::Directory(parent.to_path_buf()));
        }
        scopes.push(IgnoreScope::Exclude);
        scopes.push(IgnoreScope::Global);
        scopes
    }
}

fn normalized(path: &Path) -> (PathBuf, bool) {
    let is_dir = path.as_os_str().as_bytes().ends_with(b"/");
    (path.components().collect(), is_dir)
}

impl State {
    pub fn open_ignored(&mut self, target: Option<PathBuf>) {
        if self.repo.is_none() {
            return;
        }
        self.close_history();
        self.close_branches();
        self.close_tags();
        self.profiles_open = false;
        self.ignored.open = true;
        self.ignored.target = None;
        self.reload_ignored();
        if let Some(path) = target {
            self.select_ignored(path);
        }
    }

    pub fn close_ignored(&mut self) {
        self.ignored.open = false;
        self.ignored.target = None;
    }

    fn reload_ignored(&mut self) {
        self.ignored.request += 1;
        self.ignored.loading = true;
        self.ignored.error = None;
        self.ignored.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::IgnoreList {
                request: self.ignored.request,
            });
        }
    }

    pub fn select_ignored(&mut self, path: PathBuf) {
        let (path, is_dir) = normalized(&path);
        self.ignored.reason_request += 1;
        self.ignored.version += 1;
        self.ignored.target = Some(IgnoreTarget {
            path: path.clone(),
            is_dir,
            reason: ReasonState::Loading,
        });
        if let Some(engine) = &self.engine {
            engine.send(Command::IgnoreReason {
                request: self.ignored.reason_request,
                path,
            });
        }
    }

    pub fn ignore_add(&mut self, scope: IgnoreScope) {
        let Some(target) = self.ignored.target.clone() else {
            return;
        };
        let rule = match rule_for_path(&target.path, target.is_dir, &scope) {
            Ok(rule) => rule,
            Err(error) => {
                self.notice = Some(format!("Règle impossible : {error}"));
                return;
            }
        };
        self.send_ignore_edit(IgnoreEdit::Add { scope, rule });
    }

    pub fn ignore_remove(&mut self) {
        let Some(IgnoreTarget {
            reason: ReasonState::Known(Some(reason)),
            ..
        }) = self.ignored.target.clone()
        else {
            return;
        };
        let IgnoreSource::Scope(scope) = reason.source else {
            return;
        };
        self.send_ignore_edit(IgnoreEdit::Remove {
            scope,
            rule: reason.pattern,
        });
    }

    fn send_ignore_edit(&mut self, edit: IgnoreEdit) {
        self.ignored.edit_request += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::IgnoreEdit {
                request: self.ignored.edit_request,
                edit,
            });
        }
    }

    pub fn apply_ignore_listed(
        &mut self,
        request: u64,
        result: Result<gitpoulpe_git::IgnoredList, String>,
    ) -> bool {
        if request != self.ignored.request {
            return false;
        }
        self.ignored.loading = false;
        match result {
            Ok(list) => {
                self.ignored.paths = list.paths;
                self.ignored.truncated = list.truncated;
                self.ignored.error = None;
            }
            Err(message) => self.ignored.error = Some(message),
        }
        self.ignored.version += 1;
        true
    }

    pub fn apply_ignore_reasoned(
        &mut self,
        request: u64,
        path: &Path,
        result: Result<Option<IgnoreReason>, String>,
    ) -> bool {
        if request != self.ignored.reason_request {
            return false;
        }
        let Some(target) = self.ignored.target.as_mut().filter(|t| t.path == path) else {
            return false;
        };
        target.reason = match result {
            Ok(reason) => ReasonState::Known(reason),
            Err(message) => ReasonState::Failed(message),
        };
        self.ignored.version += 1;
        true
    }

    pub fn apply_ignore_edited(&mut self, request: u64, result: Result<String, String>) -> bool {
        if request != self.ignored.edit_request {
            return false;
        }
        match result {
            Ok(message) => {
                self.notice = Some(message);
                self.request_refresh(false);
                self.reload_ignored();
                if let Some(path) = self.ignored.target.as_ref().map(|t| {
                    let mut path = t.path.clone().into_os_string();
                    if t.is_dir {
                        path.push("/");
                    }
                    PathBuf::from(path)
                }) {
                    self.select_ignored(path);
                }
            }
            Err(message) => self.notice = Some(format!("Règle non modifiée : {message}")),
        }
        true
    }
}
