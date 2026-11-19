use std::path::{Path, PathBuf};

use gitpoulpe_core::WorkspaceBook;
use rune::{TextEdit, Tx};

use crate::state::{State, ViewKind};

pub const WORKSPACES_KEY: &str = "workspaces";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoHealth {
    Ok,
    Missing,
    NotRepository,
}

pub fn health(path: &str) -> RepoHealth {
    let path = Path::new(path);
    if !path.is_dir() {
        RepoHealth::Missing
    } else if path.join(".git").exists() {
        RepoHealth::Ok
    } else {
        RepoHealth::NotRepository
    }
}

pub fn view_name(view: ViewKind) -> &'static str {
    match view {
        ViewKind::Staged => "staged",
        _ => "unstaged",
    }
}

pub struct PendingRestore {
    pub path: String,
    pub view: String,
    pub file: Option<String>,
}

#[derive(Default)]
pub struct WorkspacesState {
    pub open: bool,
    pub book: WorkspaceBook,
    pub selected: Option<u64>,
    pub new_name: TextEdit,
    pub rename: TextEdit,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub confirm_delete: bool,
    pub pending: Option<PendingRestore>,
    pub version: u32,
}

fn text_path(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "ce chemin n'est pas en UTF-8 : il ne peut pas être enregistré".to_owned())
}

impl State {
    fn save_workspaces(&mut self) {
        let _ = self.prefs.set(WORKSPACES_KEY, &self.workspaces.book);
    }

    fn workspace_fail(&mut self, message: impl Into<String>) {
        self.workspaces.error = Some(message.into());
        self.workspaces.notice = None;
        self.workspaces.version += 1;
    }

    fn workspace_ok(&mut self, message: impl Into<String>) {
        self.workspaces.error = None;
        self.workspaces.notice = Some(message.into());
        self.workspaces.version += 1;
    }

    pub fn open_workspaces(&mut self) {
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
        self.close_favorites();
        self.close_overview();
        self.profiles_open = false;
        self.workspace_remember();
        let book = &self.workspaces.book;
        let selected = book
            .active
            .filter(|id| book.get(*id).is_some())
            .or_else(|| book.items.first().map(|w| w.id));
        self.workspaces.open = true;
        self.workspaces.selected = selected;
        self.workspaces.confirm_delete = false;
        self.workspaces.error = None;
        self.workspaces.notice = None;
        self.workspace_load_name();
        self.workspaces.version += 1;
    }

    pub fn close_workspaces(&mut self) {
        self.workspaces.open = false;
        self.workspaces.confirm_delete = false;
    }

    fn workspace_load_name(&mut self) {
        let name = self
            .workspaces
            .selected
            .and_then(|id| self.workspaces.book.get(id))
            .map(|w| w.name.clone())
            .unwrap_or_default();
        self.workspaces.rename = TextEdit::new(name);
    }

    pub fn workspace_select(&mut self, id: u64) {
        self.workspaces.selected = Some(id);
        self.workspaces.confirm_delete = false;
        self.workspaces.error = None;
        self.workspaces.notice = None;
        self.workspace_load_name();
        self.workspaces.version += 1;
    }

    pub fn workspace_create(&mut self) {
        let name = self.workspaces.new_name.text.clone();
        match self.workspaces.book.create(&name) {
            Ok(id) => {
                self.workspaces.new_name = TextEdit::new("");
                self.save_workspaces();
                self.workspace_select(id);
                self.workspace_ok("Espace créé.");
            }
            Err(error) => self.workspace_fail(error.to_string()),
        }
    }

    pub fn workspace_rename(&mut self) {
        let Some(id) = self.workspaces.selected else {
            return;
        };
        let name = self.workspaces.rename.text.clone();
        match self.workspaces.book.rename(id, &name) {
            Ok(()) => {
                self.save_workspaces();
                self.workspace_load_name();
                self.workspace_ok("Espace renommé.");
            }
            Err(error) => self.workspace_fail(error.to_string()),
        }
    }

    pub fn workspace_delete(&mut self) {
        let Some(id) = self.workspaces.selected else {
            return;
        };
        if !self.workspaces.confirm_delete {
            self.workspaces.confirm_delete = true;
            self.workspaces.version += 1;
            return;
        }
        self.workspaces.confirm_delete = false;
        match self.workspaces.book.delete(id) {
            Ok(()) => {
                self.save_workspaces();
                self.workspaces.selected = self.workspaces.book.items.first().map(|w| w.id);
                self.workspace_load_name();
                self.workspace_ok("Espace supprimé : aucun fichier de dépôt n'a été touché.");
            }
            Err(error) => self.workspace_fail(error.to_string()),
        }
    }

    pub fn workspace_cancel_delete(&mut self) {
        self.workspaces.confirm_delete = false;
        self.workspaces.version += 1;
    }

    pub fn workspace_add_current(&mut self) {
        let Some(path) = self.repo.as_ref().map(|r| r.path.clone()) else {
            self.workspace_fail("Aucun dépôt n'est ouvert : ouvrez-en un ou ajoutez un dossier.");
            return;
        };
        self.workspace_add_path(path);
    }

    pub fn workspace_add_path(&mut self, path: PathBuf) {
        let Some(id) = self.workspaces.selected else {
            self.workspace_fail("Créez ou sélectionnez d'abord un espace.");
            return;
        };
        let path = path.canonicalize().unwrap_or(path);
        let text = match text_path(&path) {
            Ok(text) => text,
            Err(message) => return self.workspace_fail(message),
        };
        if health(&text) != RepoHealth::Ok {
            return self.workspace_fail(format!("{text} n'est pas un dépôt Git."));
        }
        match self.workspaces.book.add_repo(id, &text) {
            Ok(()) => {
                self.save_workspaces();
                self.workspace_ok(format!("{text} ajouté à l'espace."));
            }
            Err(error) => self.workspace_fail(error.to_string()),
        }
    }

    pub fn workspace_add_folder(&mut self, tx: &mut Tx) {
        let options = rune::FileDialogOptions {
            title: "Ajouter un dépôt à l'espace".into(),
            ..Default::default()
        };
        tx.open_folder(options, |s: &mut State, result, _| match result {
            Ok(Some(path)) => s.workspace_add_path(path),
            Ok(None) => {}
            Err(error) => s.workspace_fail(format!("Sélecteur de dossier indisponible : {error}")),
        });
    }

    pub fn workspace_repair_folder(&mut self, old: String, tx: &mut Tx) {
        let options = rune::FileDialogOptions {
            title: "Où se trouve maintenant ce dépôt ?".into(),
            ..Default::default()
        };
        tx.open_folder(options, move |s: &mut State, result, _| match result {
            Ok(Some(path)) => s.workspace_repair_path(&old, path),
            Ok(None) => {}
            Err(error) => s.workspace_fail(format!("Sélecteur de dossier indisponible : {error}")),
        });
    }

    pub fn workspace_repair_path(&mut self, old: &str, new: PathBuf) {
        let Some(id) = self.workspaces.selected else {
            return;
        };
        let new = new.canonicalize().unwrap_or(new);
        let text = match text_path(&new) {
            Ok(text) => text,
            Err(message) => return self.workspace_fail(message),
        };
        if health(&text) != RepoHealth::Ok {
            return self.workspace_fail(format!("{text} n'est pas un dépôt Git."));
        }
        match self.workspaces.book.repair(id, old, &text) {
            Ok(()) => {
                self.save_workspaces();
                self.workspace_ok(format!("Association réparée : {text}."));
            }
            Err(error) => self.workspace_fail(error.to_string()),
        }
    }

    pub fn workspace_remove_repo(&mut self, path: &str) {
        let Some(id) = self.workspaces.selected else {
            return;
        };
        match self.workspaces.book.remove_repo(id, path) {
            Ok(()) => {
                self.save_workspaces();
                self.workspace_ok("Dépôt retiré de l'espace : aucun fichier n'a été supprimé.");
            }
            Err(error) => self.workspace_fail(error.to_string()),
        }
    }

    pub fn workspace_remember(&mut self) {
        let Some(active) = self.workspaces.book.active else {
            return;
        };
        let Some(path) = self
            .repo
            .as_ref()
            .and_then(|r| r.path.to_str().map(str::to_owned))
        else {
            return;
        };
        let file = self
            .current
            .and_then(|p| self.entry_at(p))
            .and_then(|(_, e)| e.path.to_str().map(str::to_owned));
        let view = view_name(self.view);
        self.workspaces.book.remember(active, &path, view, file);
        self.save_workspaces();
    }

    pub fn workspace_open_repo(&mut self, path: String) {
        let Some(id) = self.workspaces.selected else {
            return;
        };
        if health(&path) != RepoHealth::Ok {
            return self.workspace_fail(format!(
                "{path} est introuvable ou n'est plus un dépôt : utilisez « Réparer » pour le retrouver."
            ));
        }
        self.workspace_remember();
        self.workspaces.book.active = Some(id);
        let saved = self
            .workspaces
            .book
            .get(id)
            .and_then(|w| w.repos.iter().find(|r| r.path == path))
            .cloned();
        self.workspaces.pending = saved.map(|r| PendingRestore {
            path: path.clone(),
            view: r.view,
            file: r.file,
        });
        self.save_workspaces();
        self.close_workspaces();
        self.open_path(PathBuf::from(path));
    }

    pub fn workspace_restore(&mut self) {
        let Some(id) = self.workspaces.selected else {
            return;
        };
        let Some(workspace) = self.workspaces.book.get(id).cloned() else {
            return;
        };
        let usable: Vec<&str> = workspace
            .repos
            .iter()
            .filter(|r| health(&r.path) == RepoHealth::Ok)
            .map(|r| r.path.as_str())
            .collect();
        let missing = workspace.repos.len() - usable.len();
        let target = workspace
            .last
            .as_deref()
            .filter(|last| usable.contains(last))
            .or_else(|| usable.first().copied())
            .map(str::to_owned);
        match target {
            Some(path) => {
                self.workspace_open_repo(path);
                if missing > 0 {
                    self.notice = Some(format!(
                        "{missing} dépôt(s) de l'espace sont introuvables : réparez-les dans les espaces de travail."
                    ));
                }
            }
            None if workspace.repos.is_empty() => {
                self.workspace_fail("Cet espace ne contient aucun dépôt : ajoutez-en un.");
            }
            None => self.workspace_fail(
                "Aucun dépôt de cet espace n'est accessible : réparez les associations.",
            ),
        }
    }

    pub fn apply_workspace_restore(&mut self) {
        let Some(pending) = self.workspaces.pending.take() else {
            return;
        };
        let Some(repo) = self.repo.as_ref().map(|r| r.path.clone()) else {
            return;
        };
        let same = |a: &Path, b: &Path| match (a.canonicalize(), b.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => a == b,
        };
        if !same(&repo, Path::new(&pending.path)) {
            return;
        }
        let view = if pending.view == "staged" {
            ViewKind::Staged
        } else {
            ViewKind::Unstaged
        };
        self.set_view(view);
        if let Some(file) = pending.file
            && let Some(position) = self.files.iter().position(|&i| {
                self.entries()
                    .get(i)
                    .is_some_and(|e| e.path.to_str() == Some(file.as_str()))
            })
        {
            self.jump_to(position);
        }
    }
}
