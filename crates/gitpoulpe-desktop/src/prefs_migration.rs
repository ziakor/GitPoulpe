use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub const FORMAT_KEY: &str = "format";
pub const CURRENT_FORMAT: u64 = 1;

pub type Step = fn(&mut Map<String, Value>) -> Result<(), String>;

// Step `i` upgrades format `i` to `i + 1`; files written before versioning count as format 0.
pub const STEPS: [Step; CURRENT_FORMAT as usize] = [legacy_to_v1];

fn legacy_to_v1(_: &mut Map<String, Value>) -> Result<(), String> {
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Fresh,
    Current,
    Migrated {
        from: u64,
        backup: PathBuf,
    },
    Newer {
        found: u64,
    },
    Unreadable {
        backup: PathBuf,
        reason: String,
    },
    Failed {
        backup: Option<PathBuf>,
        reason: String,
    },
}

impl Outcome {
    pub fn writable(&self) -> bool {
        matches!(
            self,
            Self::Fresh | Self::Current | Self::Migrated { .. } | Self::Unreadable { .. }
        )
    }

    pub fn warning(&self, file: &Path) -> Option<String> {
        let file = file.display();
        match self {
            Self::Fresh | Self::Current | Self::Migrated { .. } => None,
            Self::Newer { found } => Some(format!(
                "Les préférences ({file}) viennent d'une version plus récente de GitPoulpe (format {found}, celui-ci gère jusqu'au format {CURRENT_FORMAT}) : elles sont lues sans être modifiées, vos changements ne seront pas enregistrés."
            )),
            Self::Unreadable { backup, reason } => Some(format!(
                "Les préférences ({file}) sont illisibles ({reason}) : une copie est conservée dans {} et les réglages repartent de zéro.",
                backup.display()
            )),
            Self::Failed { backup, reason } => Some(format!(
                "La migration des préférences ({file}) a échoué ({reason}) : le fichier est inchangé{}, les changements ne seront pas enregistrés.",
                backup
                    .as_ref()
                    .map(|b| format!(" et une copie est dans {}", b.display()))
                    .unwrap_or_default()
            )),
        }
    }
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn keep_copy(path: &Path, bytes: &[u8], suffix: &str) -> std::io::Result<PathBuf> {
    for n in 0.. {
        let candidate = sibling(
            path,
            &if n == 0 {
                suffix.to_owned()
            } else {
                format!("{suffix}.{n}")
            },
        );
        match std::fs::read(&candidate) {
            Ok(existing) if existing == bytes => return Ok(candidate),
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&candidate)?;
                file.write_all(bytes)?;
                file.sync_all()?;
                return Ok(candidate);
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = sibling(path, &format!(".migrating.{}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

pub fn migrate_file(path: &Path, current: u64, steps: &[Step]) -> Outcome {
    debug_assert_eq!(steps.len() as u64, current);
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Outcome::Fresh,
        Err(e) => {
            return Outcome::Failed {
                backup: None,
                reason: format!("lecture impossible : {e}"),
            };
        }
    };
    let parsed: Result<Map<String, Value>, String> = match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err("le fichier n'est pas un objet JSON".to_owned()),
        Err(e) => Err(e.to_string()),
    };
    let mut map = match parsed {
        Ok(map) => map,
        Err(reason) => {
            return match keep_copy(path, &bytes, ".unreadable") {
                Ok(backup) => Outcome::Unreadable { backup, reason },
                Err(e) => Outcome::Failed {
                    backup: None,
                    reason: format!("{reason} ; copie impossible : {e}"),
                },
            };
        }
    };
    let found = match map.get(FORMAT_KEY) {
        None => 0,
        Some(Value::Number(n)) => match n.as_u64() {
            Some(found) => found,
            None => {
                return Outcome::Failed {
                    backup: None,
                    reason: "numéro de format invalide".to_owned(),
                };
            }
        },
        Some(_) => {
            return Outcome::Failed {
                backup: None,
                reason: "numéro de format invalide".to_owned(),
            };
        }
    };
    if found > current {
        return Outcome::Newer { found };
    }
    if found == current {
        return Outcome::Current;
    }
    let backup = match keep_copy(path, &bytes, &format!(".v{found}.bak")) {
        Ok(backup) => backup,
        Err(e) => {
            return Outcome::Failed {
                backup: None,
                reason: format!("copie de sauvegarde impossible : {e}"),
            };
        }
    };
    for (index, step) in steps.iter().enumerate().skip(found as usize) {
        if let Err(reason) = step(&mut map) {
            return Outcome::Failed {
                backup: Some(backup),
                reason: format!("étape {} → {} : {reason}", index, index + 1),
            };
        }
    }
    map.insert(FORMAT_KEY.to_owned(), Value::from(current));
    let json = match serde_json::to_vec_pretty(&map) {
        Ok(json) => json,
        Err(e) => {
            return Outcome::Failed {
                backup: Some(backup),
                reason: e.to_string(),
            };
        }
    };
    match write_atomic(path, &json) {
        Ok(()) => Outcome::Migrated {
            from: found,
            backup,
        },
        Err(e) => Outcome::Failed {
            backup: Some(backup),
            reason: format!("écriture impossible : {e}"),
        },
    }
}

#[cfg(unix)]
pub fn prefs_path(app_id: &str) -> Option<PathBuf> {
    let var = |k: &str| std::env::var_os(k);
    if let Some(dir) = var("RUNE_PREFS_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir).join(app_id).join("prefs.json"));
    }
    rune::dirs::AppDirs::new(app_id).map(|d| d.config().join("prefs.json"))
}

pub fn open_at(path: &Path) -> (rune::prefs::Prefs, Option<String>) {
    use rune::prefs::Prefs;
    let outcome = migrate_file(path, CURRENT_FORMAT, &STEPS);
    let warning = outcome.warning(path);
    if outcome.writable() {
        let mut prefs = Prefs::at(path);
        if !prefs.contains(FORMAT_KEY) {
            let _ = prefs.set(FORMAT_KEY, &CURRENT_FORMAT);
        }
        return (prefs, warning);
    }
    let mut memory = Prefs::memory();
    if let Ok(bytes) = std::fs::read(path)
        && let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&bytes)
    {
        for (key, value) in map {
            let _ = memory.set(&key, &value);
        }
    }
    (memory, warning)
}

#[cfg(unix)]
pub fn open_prefs(app_id: &str) -> (rune::prefs::Prefs, Option<String>) {
    match prefs_path(app_id) {
        Some(path) => open_at(&path),
        None => (rune::prefs::Prefs::open(app_id), None),
    }
}

#[cfg(not(unix))]
pub fn open_prefs(app_id: &str) -> (rune::prefs::Prefs, Option<String>) {
    (rune::prefs::Prefs::open(app_id), None)
}
