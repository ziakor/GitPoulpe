use std::path::PathBuf;

use gitpoulpe_core::{Favorite, FavoriteBook};
use gitpoulpe_hosting::RepoSummary;
use rune::{TextEdit, Tx};

use crate::accounts::Status;
use crate::palette::fold;
use crate::state::State;
use crate::workspaces::{RepoHealth, health};

pub const FAVORITES_KEY: &str = "favorites";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteAccess {
    NoAccount,
    Unchecked(String),
    Checking(String),
    Verified(String),
    Revoked(String, String),
}

#[derive(Default)]
pub struct FavoritesState {
    pub open: bool,
    pub book: FavoriteBook,
    pub query: TextEdit,
    pub selected: Option<String>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub version: u32,
}

impl State {
    fn save_favorites(&mut self) {
        let _ = self.prefs.set(FAVORITES_KEY, &self.favorites.book);
    }

    fn favorite_fail(&mut self, message: impl Into<String>) {
        self.favorites.error = Some(message.into());
        self.favorites.notice = None;
        self.favorites.version += 1;
    }

    fn favorite_ok(&mut self, message: impl Into<String>) {
        self.favorites.error = None;
        self.favorites.notice = Some(message.into());
        self.favorites.version += 1;
    }

    pub fn open_favorites(&mut self) {
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
        self.close_overview();
        self.close_notices();
        self.close_issues();
        self.accounts.open = false;
        self.profiles_open = false;
        self.favorites.open = true;
        self.favorites.error = None;
        self.favorites.notice = None;
        self.favorites.query = TextEdit::new("");
        self.favorites.version += 1;
    }

    pub fn close_favorites(&mut self) {
        self.favorites.open = false;
    }

    pub fn favorites_visible(&self) -> Vec<&Favorite> {
        let needle = fold(&self.favorites.query.text);
        let tokens: Vec<&str> = needle.split_whitespace().collect();
        self.favorites
            .book
            .items
            .iter()
            .filter(|f| {
                let extra = match f {
                    Favorite::Local { path, .. } => path.clone(),
                    Favorite::Remote { clone_url, .. } => clone_url.clone(),
                };
                let hay = format!("{} {}", fold(f.label()), fold(&extra));
                tokens.iter().all(|t| hay.contains(t))
            })
            .collect()
    }

    pub fn favorite_remote_access(&self, connection: &str) -> RemoteAccess {
        let Some(account) = self.accounts.book.get(connection) else {
            return RemoteAccess::NoAccount;
        };
        let who = format!("{}@{}", account.login, account.host_caption());
        match self.accounts.statuses.get(connection) {
            None => RemoteAccess::Unchecked(who),
            Some(Status::Checking) => RemoteAccess::Checking(who),
            Some(Status::Ok(_)) => RemoteAccess::Verified(who),
            Some(Status::Failed(why)) => RemoteAccess::Revoked(who, why.clone()),
        }
    }

    pub fn favorites_add_current(&mut self) {
        let Some((path, name)) = self.repo.as_ref().map(|r| (r.path.clone(), r.name.clone()))
        else {
            return self.favorite_fail("Aucun dépôt n'est ouvert.");
        };
        self.favorites_add_path(path, Some(name));
    }

    pub fn favorites_add_path(&mut self, path: PathBuf, name: Option<String>) {
        let path = path.canonicalize().unwrap_or(path);
        let Some(text) = path.to_str().map(str::to_owned) else {
            return self
                .favorite_fail("Ce chemin n'est pas en UTF-8 : il ne peut pas être enregistré.");
        };
        if health(&text) != RepoHealth::Ok {
            return self.favorite_fail(format!("{text} n'est pas un dépôt Git."));
        }
        let name = name.unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| text.clone())
        });
        match self.favorites.book.add(Favorite::Local {
            path: text,
            name: name.clone(),
        }) {
            Ok(()) => {
                self.save_favorites();
                self.favorite_ok(format!("{name} ajouté aux favoris."));
            }
            Err(error) => self.favorite_fail(error.to_string()),
        }
    }

    pub fn favorites_add_remote(&mut self, repo: &RepoSummary) {
        let favorite = Favorite::Remote {
            connection: repo.connection_id.clone(),
            full_name: repo.full_name(),
            clone_url: repo.clone_url.clone(),
        };
        match self.favorites.book.add(favorite) {
            Ok(()) => {
                self.save_favorites();
                self.favorite_ok(format!("{} ajouté aux favoris.", repo.full_name()));
            }
            Err(error) => self.favorite_fail(error.to_string()),
        }
    }

    pub fn favorite_remove(&mut self, key: &str) {
        match self.favorites.book.remove(key) {
            Ok(()) => {
                if self.favorites.selected.as_deref() == Some(key) {
                    self.favorites.selected = None;
                }
                self.save_favorites();
                self.favorite_ok("Favori retiré : le dépôt et la connexion ne sont pas touchés.");
            }
            Err(error) => self.favorite_fail(error.to_string()),
        }
    }

    pub fn favorite_open(&mut self, key: &str) {
        let Some(favorite) = self
            .favorites
            .book
            .items
            .iter()
            .find(|f| f.key() == key)
            .cloned()
        else {
            return;
        };
        match favorite {
            Favorite::Local { path, name } => {
                if health(&path) != RepoHealth::Ok {
                    return self.favorite_fail(format!(
                        "{name} est introuvable à {path} : utilisez « Réparer… » pour indiquer son nouvel emplacement."
                    ));
                }
                self.close_favorites();
                self.open_path(PathBuf::from(path));
            }
            Favorite::Remote {
                connection,
                full_name,
                clone_url,
            } => match self.favorite_remote_access(&connection) {
                RemoteAccess::NoAccount => self.favorite_fail(format!(
                    "Le compte qui donnait accès à {full_name} n'est plus connecté : reconnectez-le dans « Comptes fournisseurs »."
                )),
                RemoteAccess::Revoked(who, why) => self.favorite_fail(format!(
                    "L'accès de {who} à {full_name} est refusé ({why}) : reconnectez le compte dans « Comptes fournisseurs »."
                )),
                _ => {
                    self.close_favorites();
                    self.begin_clone();
                    self.clone.url = TextEdit::new(clone_url);
                }
            },
        }
    }

    pub fn favorite_check(&mut self, key: &str) {
        let connection = self
            .favorites
            .book
            .items
            .iter()
            .find(|f| f.key() == key)
            .and_then(|f| match f {
                Favorite::Remote { connection, .. } => Some(connection.clone()),
                Favorite::Local { .. } => None,
            });
        if let Some(connection) = connection {
            if self.accounts.book.get(&connection).is_none() {
                return self.favorite_fail("Ce compte n'est plus connecté.");
            }
            self.check_account(&connection);
            self.favorites.version += 1;
        }
    }

    pub fn favorite_repair_folder(&mut self, key: String, tx: &mut Tx) {
        let options = rune::FileDialogOptions {
            title: "Où se trouve maintenant ce dépôt ?".into(),
            ..Default::default()
        };
        tx.open_folder(options, move |s: &mut State, result, _| match result {
            Ok(Some(path)) => s.favorite_repair_path(&key, path),
            Ok(None) => {}
            Err(error) => s.favorite_fail(format!("Sélecteur de dossier indisponible : {error}")),
        });
    }

    pub fn favorite_repair_path(&mut self, key: &str, new: PathBuf) {
        let new = new.canonicalize().unwrap_or(new);
        let Some(text) = new.to_str().map(str::to_owned) else {
            return self.favorite_fail("Ce chemin n'est pas en UTF-8.");
        };
        if health(&text) != RepoHealth::Ok {
            return self.favorite_fail(format!("{text} n'est pas un dépôt Git."));
        }
        let name = new
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| text.clone());
        match self.favorites.book.repair_local(key, &text, &name) {
            Ok(()) => {
                self.save_favorites();
                self.favorites.selected = Some(
                    Favorite::Local {
                        path: text.clone(),
                        name,
                    }
                    .key(),
                );
                self.favorite_ok(format!("Favori réparé : {text}."));
            }
            Err(error) => self.favorite_fail(error.to_string()),
        }
    }
}
