use rune::prefs::Prefs;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::prefs_migration::{CURRENT_FORMAT, FORMAT_KEY, Outcome};
use crate::state::State;

enum Disk {
    Fresh(Prefs),
    Newer(u64),
    Unusable,
}

fn reread(prefs: &Prefs) -> Disk {
    let Some(path) = prefs.path() else {
        return Disk::Unusable;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return Disk::Unusable;
    };
    let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&bytes) else {
        return Disk::Unusable;
    };
    match map.get(FORMAT_KEY).and_then(Value::as_u64).unwrap_or(0) {
        found if found > CURRENT_FORMAT => Disk::Newer(found),
        _ => Disk::Fresh(Prefs::at(path)),
    }
}

#[derive(PartialEq, Eq)]
enum Refresh {
    Reread,
    Kept,
    Frozen,
}

impl State {
    fn refresh_prefs(&mut self) -> Refresh {
        match reread(&self.prefs) {
            Disk::Fresh(fresh) => {
                self.prefs = fresh;
                Refresh::Reread
            }
            Disk::Newer(found) => {
                if let Some(path) = self.prefs.path() {
                    self.prefs_warning = Outcome::Newer { found }.warning(path);
                }
                Refresh::Frozen
            }
            Disk::Unusable => Refresh::Kept,
        }
    }

    pub fn reload_prefs(&mut self) {
        self.refresh_prefs();
    }

    pub fn save_pref<T: Serialize + ?Sized>(&mut self, key: &str, value: &T) {
        if self.refresh_prefs() != Refresh::Frozen {
            let _ = self.prefs.set(key, value);
        }
    }

    pub fn update_pref<T: Serialize + DeserializeOwned>(
        &mut self,
        key: &str,
        local: T,
        change: impl FnOnce(&mut T),
    ) -> T {
        let refresh = self.refresh_prefs();
        let mut value = match refresh {
            Refresh::Reread => self.prefs.get(key).unwrap_or(local),
            Refresh::Kept | Refresh::Frozen => local,
        };
        change(&mut value);
        if refresh != Refresh::Frozen {
            let _ = self.prefs.set(key, &value);
        }
        value
    }
}
