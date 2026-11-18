use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SigningPreference {
    None,
    Ssh { public_key: PathBuf },
    Gpg { key_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub label: String,
    pub user_name: String,
    pub user_email: String,
    pub signing: SigningPreference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileError {
    EmptyLabel,
    DuplicateLabel,
    EmptyName,
    InvalidName,
    InvalidEmail,
    UnknownProfile,
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::EmptyLabel => "Donne un nom au profil.",
            Self::DuplicateLabel => "Un profil porte déjà ce nom.",
            Self::EmptyName => "Indique le nom Git (user.name).",
            Self::InvalidName => {
                "Le nom Git ne peut contenir ni retour à la ligne ni les caractères < et >."
            }
            Self::InvalidEmail => "Indique une adresse e-mail valide (user.email).",
            Self::UnknownProfile => "Ce profil n'existe plus.",
        })
    }
}

impl std::error::Error for ProfileError {}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileBook {
    pub profiles: Vec<Profile>,
    pub assignments: BTreeMap<String, String>,
}

fn clean(text: &str) -> bool {
    !text.chars().any(|c| c.is_control() || c == '<' || c == '>')
}

impl Profile {
    pub fn validate(&self) -> Result<(), ProfileError> {
        if self.label.trim().is_empty() {
            return Err(ProfileError::EmptyLabel);
        }
        if self.user_name.trim().is_empty() {
            return Err(ProfileError::EmptyName);
        }
        if !clean(&self.user_name) {
            return Err(ProfileError::InvalidName);
        }
        let email = self.user_email.trim();
        let valid_email = email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.contains('.') && !domain.ends_with('.')
        }) && clean(email)
            && !email.contains(char::is_whitespace);
        if !valid_email {
            return Err(ProfileError::InvalidEmail);
        }
        Ok(())
    }
}

impl ProfileBook {
    pub fn repo_key(common_dir: &std::path::Path) -> String {
        common_dir.to_string_lossy().into_owned()
    }

    pub fn new_id(&self) -> String {
        (1..)
            .map(|n| format!("p{n}"))
            .find(|id| self.profiles.iter().all(|p| p.id != *id))
            .unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    pub fn profile_for(&self, repo_key: &str) -> Option<&Profile> {
        self.assignments.get(repo_key).and_then(|id| self.get(id))
    }

    pub fn upsert(&mut self, mut profile: Profile) -> Result<(), ProfileError> {
        profile.label = profile.label.trim().to_owned();
        profile.user_name = profile.user_name.trim().to_owned();
        profile.user_email = profile.user_email.trim().to_owned();
        profile.validate()?;
        let same_label = |p: &&Profile| p.label.to_lowercase() == profile.label.to_lowercase();
        if self
            .profiles
            .iter()
            .filter(same_label)
            .any(|p| p.id != profile.id)
        {
            return Err(ProfileError::DuplicateLabel);
        }
        match self.profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => *existing = profile,
            None => self.profiles.push(profile),
        }
        Ok(())
    }

    pub fn remove(&mut self, id: &str) {
        self.profiles.retain(|p| p.id != id);
        self.assignments.retain(|_, assigned| assigned != id);
    }

    pub fn assign(&mut self, repo_key: &str, id: Option<&str>) -> Result<(), ProfileError> {
        match id {
            Some(id) if self.get(id).is_none() => Err(ProfileError::UnknownProfile),
            Some(id) => {
                self.assignments.insert(repo_key.to_owned(), id.to_owned());
                Ok(())
            }
            None => {
                self.assignments.remove(repo_key);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, label: &str) -> Profile {
        Profile {
            id: id.to_owned(),
            label: label.to_owned(),
            user_name: "Ada Lovelace".to_owned(),
            user_email: "ada@example.org".to_owned(),
            signing: SigningPreference::None,
        }
    }

    #[test]
    fn two_repositories_keep_their_own_profile() {
        let mut book = ProfileBook::default();
        book.upsert(profile("p1", "Perso")).unwrap();
        book.upsert(Profile {
            user_email: "ada@work.example".into(),
            ..profile("p2", "Travail")
        })
        .unwrap();
        book.assign("/repos/a", Some("p1")).unwrap();
        book.assign("/repos/b", Some("p2")).unwrap();
        assert_eq!(book.profile_for("/repos/a").unwrap().label, "Perso");
        assert_eq!(
            book.profile_for("/repos/b").unwrap().user_email,
            "ada@work.example"
        );
        assert!(book.profile_for("/repos/c").is_none());
    }

    #[test]
    fn invalid_profiles_are_rejected_with_a_specific_reason() {
        let mut book = ProfileBook::default();
        let bad = |change: &dyn Fn(&mut Profile)| {
            let mut p = profile("p1", "Perso");
            change(&mut p);
            p
        };
        assert_eq!(
            book.upsert(bad(&|p| p.label = "  ".into())),
            Err(ProfileError::EmptyLabel)
        );
        assert_eq!(
            book.upsert(bad(&|p| p.user_name = String::new())),
            Err(ProfileError::EmptyName)
        );
        assert_eq!(
            book.upsert(bad(&|p| p.user_name = "Ada\nLovelace".into())),
            Err(ProfileError::InvalidName)
        );
        assert_eq!(
            book.upsert(bad(&|p| p.user_name = "Ada <x>".into())),
            Err(ProfileError::InvalidName)
        );
        for email in [
            "",
            "no-at-sign",
            "a@b",
            "@example.org",
            "a b@example.org",
            "a@example.",
            "a\n@x.org",
        ] {
            assert_eq!(
                book.upsert(bad(&|p| p.user_email = email.into())),
                Err(ProfileError::InvalidEmail),
                "{email:?}"
            );
        }
        assert!(book.profiles.is_empty());
        book.upsert(bad(&|p| p.user_email = " ada@example.org\n".into()))
            .unwrap();
        assert_eq!(
            book.profiles[0].user_email, "ada@example.org",
            "the border is trimmed"
        );
    }

    #[test]
    fn labels_are_unique_ignoring_case_but_a_profile_can_keep_its_own() {
        let mut book = ProfileBook::default();
        book.upsert(profile("p1", "Perso")).unwrap();
        assert_eq!(
            book.upsert(profile("p2", "PERSO")),
            Err(ProfileError::DuplicateLabel)
        );
        book.upsert(Profile {
            user_name: "Renamed".into(),
            ..profile("p1", "Perso")
        })
        .unwrap();
        assert_eq!(book.profiles.len(), 1);
        assert_eq!(book.profiles[0].user_name, "Renamed");
    }

    #[test]
    fn removing_a_profile_drops_its_assignments_and_unknown_ones_cannot_be_assigned() {
        let mut book = ProfileBook::default();
        book.upsert(profile("p1", "Perso")).unwrap();
        book.assign("/repos/a", Some("p1")).unwrap();
        assert_eq!(
            book.assign("/repos/a", Some("nope")),
            Err(ProfileError::UnknownProfile)
        );
        book.remove("p1");
        assert!(book.assignments.is_empty());
        assert_eq!(book.new_id(), "p1");
    }

    #[test]
    fn the_book_round_trips_through_json_without_loss() {
        let mut book = ProfileBook::default();
        book.upsert(Profile {
            signing: SigningPreference::Ssh {
                public_key: "/home/ada/.ssh/id_ed25519.pub".into(),
            },
            ..profile("p1", "Perso")
        })
        .unwrap();
        book.assign("/repos/a", Some("p1")).unwrap();
        let json = serde_json::to_string(&book).unwrap();
        assert_eq!(serde_json::from_str::<ProfileBook>(&json).unwrap(), book);
    }
}
