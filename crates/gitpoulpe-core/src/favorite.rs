use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Favorite {
    Local {
        path: String,
        name: String,
    },
    Remote {
        connection: String,
        full_name: String,
        clone_url: String,
    },
}

impl Favorite {
    pub fn key(&self) -> String {
        match self {
            Self::Local { path, .. } => format!("local:{path}"),
            Self::Remote {
                connection,
                full_name,
                ..
            } => format!("remote:{connection}:{}", full_name.to_lowercase()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Local { name, .. } => name,
            Self::Remote { full_name, .. } => full_name,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FavoriteBook {
    pub items: Vec<Favorite>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FavoriteError {
    AlreadyThere(String),
    Unknown,
}

impl std::fmt::Display for FavoriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyThere(label) => write!(f, "{label} est déjà dans les favoris"),
            Self::Unknown => f.write_str("ce favori n'existe plus"),
        }
    }
}

impl std::error::Error for FavoriteError {}

impl FavoriteBook {
    pub fn add(&mut self, favorite: Favorite) -> Result<(), FavoriteError> {
        if self.items.iter().any(|f| f.key() == favorite.key()) {
            return Err(FavoriteError::AlreadyThere(favorite.label().to_owned()));
        }
        self.items.push(favorite);
        Ok(())
    }

    pub fn remove(&mut self, key: &str) -> Result<(), FavoriteError> {
        let before = self.items.len();
        self.items.retain(|f| f.key() != key);
        if self.items.len() == before {
            return Err(FavoriteError::Unknown);
        }
        Ok(())
    }

    pub fn repair_local(&mut self, key: &str, path: &str, name: &str) -> Result<(), FavoriteError> {
        let replacement = Favorite::Local {
            path: path.to_owned(),
            name: name.to_owned(),
        };
        if replacement.key() != key && self.items.iter().any(|f| f.key() == replacement.key()) {
            return Err(FavoriteError::AlreadyThere(name.to_owned()));
        }
        let slot = self
            .items
            .iter_mut()
            .find(|f| f.key() == key)
            .ok_or(FavoriteError::Unknown)?;
        *slot = replacement;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(path: &str) -> Favorite {
        Favorite::Local {
            path: path.to_owned(),
            name: path.rsplit('/').next().unwrap().to_owned(),
        }
    }

    fn remote(connection: &str, name: &str) -> Favorite {
        Favorite::Remote {
            connection: connection.to_owned(),
            full_name: name.to_owned(),
            clone_url: format!("https://example.test/{name}.git"),
        }
    }

    #[test]
    fn a_favorite_is_added_once_and_removed_by_its_key() {
        let mut book = FavoriteBook::default();
        book.add(local("/a/proj")).unwrap();
        book.add(remote("gh/octo", "acme/tool")).unwrap();
        assert_eq!(
            book.add(local("/a/proj")),
            Err(FavoriteError::AlreadyThere("proj".to_owned()))
        );
        assert_eq!(
            book.add(remote("gh/octo", "ACME/Tool")),
            Err(FavoriteError::AlreadyThere("ACME/Tool".to_owned()))
        );
        book.add(remote("gh/other", "acme/tool")).unwrap();
        assert_eq!(book.items.len(), 3);
        book.remove("local:/a/proj").unwrap();
        assert_eq!(book.remove("local:/a/proj"), Err(FavoriteError::Unknown));
        assert_eq!(book.items.len(), 2);
    }

    #[test]
    fn a_local_favorite_can_be_repointed_without_creating_a_duplicate() {
        let mut book = FavoriteBook::default();
        book.add(local("/a/one")).unwrap();
        book.add(local("/a/two")).unwrap();
        assert_eq!(
            book.repair_local("local:/a/one", "/a/two", "two"),
            Err(FavoriteError::AlreadyThere("two".to_owned()))
        );
        book.repair_local("local:/a/one", "/b/one", "one").unwrap();
        assert_eq!(book.items[0], local("/b/one"));
        assert_eq!(
            book.repair_local("local:/zz", "/c", "c"),
            Err(FavoriteError::Unknown)
        );
    }

    #[test]
    fn the_stored_form_round_trips_and_an_empty_file_loads() {
        let mut book = FavoriteBook::default();
        book.add(local("/a/proj")).unwrap();
        book.add(remote("gh/octo", "acme/tool")).unwrap();
        let text = serde_json::to_string(&book).unwrap();
        assert_eq!(serde_json::from_str::<FavoriteBook>(&text).unwrap(), book);
        assert_eq!(
            serde_json::from_str::<FavoriteBook>("{}").unwrap(),
            FavoriteBook::default()
        );
    }
}
