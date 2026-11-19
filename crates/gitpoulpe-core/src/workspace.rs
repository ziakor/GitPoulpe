use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRepo {
    pub path: String,
    #[serde(default)]
    pub view: String,
    #[serde(default)]
    pub file: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub repos: Vec<WorkspaceRepo>,
    #[serde(default)]
    pub last: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceBook {
    pub items: Vec<Workspace>,
    pub next_id: u64,
    pub active: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceError {
    EmptyName,
    NameTaken(String),
    Unknown,
    AlreadyThere(String),
    NotThere,
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => f.write_str("donnez un nom à l'espace de travail"),
            Self::NameTaken(name) => write!(f, "un espace nommé « {name} » existe déjà"),
            Self::Unknown => f.write_str("cet espace de travail n'existe plus"),
            Self::AlreadyThere(path) => write!(f, "{path} est déjà dans cet espace"),
            Self::NotThere => f.write_str("ce dépôt n'est pas dans l'espace"),
        }
    }
}

impl std::error::Error for WorkspaceError {}

impl WorkspaceBook {
    pub fn get(&self, id: u64) -> Option<&Workspace> {
        self.items.iter().find(|w| w.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Result<&mut Workspace, WorkspaceError> {
        self.items
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or(WorkspaceError::Unknown)
    }

    fn check_name(&self, name: &str, except: Option<u64>) -> Result<String, WorkspaceError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(WorkspaceError::EmptyName);
        }
        if self
            .items
            .iter()
            .any(|w| Some(w.id) != except && w.name.to_lowercase() == name.to_lowercase())
        {
            return Err(WorkspaceError::NameTaken(name.to_owned()));
        }
        Ok(name.to_owned())
    }

    pub fn create(&mut self, name: &str) -> Result<u64, WorkspaceError> {
        let name = self.check_name(name, None)?;
        let id = self
            .items
            .iter()
            .map(|w| w.id)
            .max()
            .unwrap_or(0)
            .max(self.next_id)
            + 1;
        self.next_id = id;
        self.items.push(Workspace {
            id,
            name,
            ..Workspace::default()
        });
        Ok(id)
    }

    pub fn rename(&mut self, id: u64, name: &str) -> Result<(), WorkspaceError> {
        let name = self.check_name(name, Some(id))?;
        self.get_mut(id)?.name = name;
        Ok(())
    }

    pub fn delete(&mut self, id: u64) -> Result<(), WorkspaceError> {
        let before = self.items.len();
        self.items.retain(|w| w.id != id);
        if self.items.len() == before {
            return Err(WorkspaceError::Unknown);
        }
        if self.active == Some(id) {
            self.active = None;
        }
        Ok(())
    }

    pub fn add_repo(&mut self, id: u64, path: &str) -> Result<(), WorkspaceError> {
        let workspace = self.get_mut(id)?;
        if workspace.repos.iter().any(|r| r.path == path) {
            return Err(WorkspaceError::AlreadyThere(path.to_owned()));
        }
        workspace.repos.push(WorkspaceRepo {
            path: path.to_owned(),
            view: "unstaged".to_owned(),
            file: None,
        });
        Ok(())
    }

    pub fn remove_repo(&mut self, id: u64, path: &str) -> Result<(), WorkspaceError> {
        let workspace = self.get_mut(id)?;
        let before = workspace.repos.len();
        workspace.repos.retain(|r| r.path != path);
        if workspace.repos.len() == before {
            return Err(WorkspaceError::NotThere);
        }
        if workspace.last.as_deref() == Some(path) {
            workspace.last = None;
        }
        Ok(())
    }

    pub fn repair(&mut self, id: u64, old: &str, new: &str) -> Result<(), WorkspaceError> {
        let workspace = self.get_mut(id)?;
        if old != new && workspace.repos.iter().any(|r| r.path == new) {
            return Err(WorkspaceError::AlreadyThere(new.to_owned()));
        }
        let repo = workspace
            .repos
            .iter_mut()
            .find(|r| r.path == old)
            .ok_or(WorkspaceError::NotThere)?;
        repo.path = new.to_owned();
        if workspace.last.as_deref() == Some(old) {
            workspace.last = Some(new.to_owned());
        }
        Ok(())
    }

    pub fn remember(&mut self, id: u64, path: &str, view: &str, file: Option<String>) {
        if let Ok(workspace) = self.get_mut(id)
            && let Some(repo) = workspace.repos.iter_mut().find(|r| r.path == path)
        {
            repo.view = view.to_owned();
            repo.file = file;
            workspace.last = Some(path.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_required_and_unique_ignoring_case() {
        let mut book = WorkspaceBook::default();
        let first = book.create("  Travail  ").unwrap();
        assert_eq!(book.get(first).unwrap().name, "Travail");
        assert_eq!(book.create("  "), Err(WorkspaceError::EmptyName));
        assert_eq!(
            book.create("TRAVAIL"),
            Err(WorkspaceError::NameTaken("TRAVAIL".to_owned()))
        );
        let second = book.create("Perso").unwrap();
        assert_ne!(first, second);
        assert_eq!(book.rename(second, "perso"), Ok(()));
        assert_eq!(
            book.rename(second, "travail"),
            Err(WorkspaceError::NameTaken("travail".to_owned()))
        );
        assert_eq!(book.rename(99, "x"), Err(WorkspaceError::Unknown));
    }

    #[test]
    fn identifiers_are_never_reused_after_a_deletion() {
        let mut book = WorkspaceBook::default();
        let a = book.create("a").unwrap();
        let b = book.create("b").unwrap();
        book.delete(b).unwrap();
        let c = book.create("c").unwrap();
        assert!(c > b && b > a);
        assert_eq!(book.delete(b), Err(WorkspaceError::Unknown));
    }

    #[test]
    fn repositories_are_added_once_removed_and_repaired_without_touching_the_others() {
        let mut book = WorkspaceBook::default();
        let id = book.create("w").unwrap();
        book.add_repo(id, "/a").unwrap();
        book.add_repo(id, "/b").unwrap();
        assert_eq!(
            book.add_repo(id, "/a"),
            Err(WorkspaceError::AlreadyThere("/a".to_owned()))
        );
        book.remember(id, "/b", "staged", Some("x.rs".to_owned()));
        assert_eq!(book.get(id).unwrap().last.as_deref(), Some("/b"));
        book.repair(id, "/b", "/moved").unwrap();
        let workspace = book.get(id).unwrap();
        assert_eq!(workspace.last.as_deref(), Some("/moved"));
        assert_eq!(
            (
                workspace.repos[1].view.as_str(),
                workspace.repos[1].file.as_deref()
            ),
            ("staged", Some("x.rs"))
        );
        assert_eq!(workspace.repos[0].path, "/a");
        assert_eq!(
            book.repair(id, "/moved", "/a"),
            Err(WorkspaceError::AlreadyThere("/a".to_owned()))
        );
        assert_eq!(
            book.repair(id, "/nope", "/z"),
            Err(WorkspaceError::NotThere)
        );
        book.remove_repo(id, "/moved").unwrap();
        assert!(book.get(id).unwrap().last.is_none());
        assert_eq!(
            book.remove_repo(id, "/moved"),
            Err(WorkspaceError::NotThere)
        );
    }

    #[test]
    fn deleting_the_active_workspace_clears_it_and_old_files_still_load() {
        let mut book = WorkspaceBook::default();
        let id = book.create("w").unwrap();
        book.active = Some(id);
        book.delete(id).unwrap();
        assert_eq!(book.active, None);
        let old: WorkspaceBook =
            serde_json::from_str(r#"{"items":[{"id":3,"name":"x"}]}"#).unwrap();
        assert_eq!(old.items[0].repos.len(), 0);
        assert_eq!(old.next_id, 0);
        let again = old.clone();
        let mut again = again;
        assert_eq!(again.create("y").unwrap(), 4);
    }
}
