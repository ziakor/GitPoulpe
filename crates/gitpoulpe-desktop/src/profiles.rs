use std::path::PathBuf;

use gitpoulpe_core::{Profile, SigningPreference};
use gitpoulpe_git::{Identity, SigningCheck};
use rune::TextEdit;

pub const PROFILES_KEY: &str = "profiles";

pub struct IdentityShown {
    pub profile: Option<String>,
    pub result: Result<Identity, String>,
    pub signing: SigningCheck,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigningKind {
    None,
    Ssh,
    Gpg,
}

pub struct ProfileForm {
    pub label: TextEdit,
    pub name: TextEdit,
    pub email: TextEdit,
    pub key: TextEdit,
    pub signing: SigningKind,
    pub editing: Option<String>,
    pub error: Option<String>,
    pub signing_check: Option<SigningCheck>,
}

impl Default for ProfileForm {
    fn default() -> Self {
        Self {
            label: TextEdit::new(""),
            name: TextEdit::new(""),
            email: TextEdit::new(""),
            key: TextEdit::new(""),
            signing: SigningKind::None,
            editing: None,
            error: None,
            signing_check: None,
        }
    }
}

pub fn expand_home(text: &str) -> PathBuf {
    let text = text.trim();
    match (text.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(text),
    }
}

impl ProfileForm {
    pub fn load(&mut self, profile: &Profile) {
        *self = Self::default();
        self.label = TextEdit::new(&profile.label);
        self.name = TextEdit::new(&profile.user_name);
        self.email = TextEdit::new(&profile.user_email);
        self.editing = Some(profile.id.clone());
        match &profile.signing {
            SigningPreference::None => {}
            SigningPreference::Ssh { public_key } => {
                self.signing = SigningKind::Ssh;
                self.key = TextEdit::new(public_key.to_string_lossy());
            }
            SigningPreference::Gpg { key_id } => {
                self.signing = SigningKind::Gpg;
                self.key = TextEdit::new(key_id);
            }
        }
    }

    pub fn to_profile(&self, id: String) -> Profile {
        let reference = self.key.text.trim().to_owned();
        Profile {
            id,
            label: self.label.text.clone(),
            user_name: self.name.text.clone(),
            user_email: self.email.text.clone(),
            signing: match self.signing {
                SigningKind::None => SigningPreference::None,
                SigningKind::Ssh => SigningPreference::Ssh {
                    public_key: expand_home(&reference),
                },
                SigningKind::Gpg => SigningPreference::Gpg { key_id: reference },
            },
        }
    }
}
