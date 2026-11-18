use std::collections::HashMap;
use std::sync::Mutex;

const SERVICE: &str = "gitpoulpe";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretError(pub String);

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "trousseau système indisponible : {}", self.0)
    }
}

impl std::error::Error for SecretError {}

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, key: &str, value: &str) -> Result<(), SecretError>;
    fn delete(&self, key: &str) -> Result<(), SecretError>;
}

#[derive(Default)]
pub struct MemoryStore {
    values: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.values.lock().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        Ok(self.values.lock().unwrap().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
        self.values
            .lock()
            .unwrap()
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

#[derive(Default)]
pub struct KeyringStore;

fn entry(key: &str) -> Result<keyring::Entry, SecretError> {
    keyring::Entry::new(SERVICE, key).map_err(|e| SecretError(e.to_string()))
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        match entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError(e.to_string())),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
        entry(key)?
            .set_password(value)
            .map_err(|e| SecretError(e.to_string()))
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        match entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError(e.to_string())),
        }
    }
}
