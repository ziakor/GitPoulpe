use std::collections::BTreeMap;

use rune::{Key, KeyEvent, TextEdit};

use crate::state::State;

pub const SHORTCUTS_KEY: &str = "shortcuts";

const RESERVED: [&str; 7] = [
    "Ctrl+C",
    "Ctrl+V",
    "Ctrl+X",
    "Ctrl+A",
    "Ctrl+Z",
    "Ctrl+Y",
    "Ctrl+Shift+Z",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Reading,
    Global,
}

pub struct ShortcutAction {
    pub id: &'static str,
    pub label: &'static str,
    pub default: &'static str,
    pub scope: Scope,
}

pub const ACTIONS: &[ShortcutAction] = &[
    ShortcutAction {
        id: "filter",
        label: "Filtrer la liste des fichiers",
        default: "/",
        scope: Scope::Reading,
    },
    ShortcutAction {
        id: "next-file",
        label: "Fichier suivant",
        default: "j",
        scope: Scope::Reading,
    },
    ShortcutAction {
        id: "prev-file",
        label: "Fichier précédent",
        default: "k",
        scope: Scope::Reading,
    },
    ShortcutAction {
        id: "view-unstaged",
        label: "Vue des fichiers non indexés",
        default: "1",
        scope: Scope::Reading,
    },
    ShortcutAction {
        id: "view-staged",
        label: "Vue des fichiers indexés",
        default: "2",
        scope: Scope::Reading,
    },
    ShortcutAction {
        id: "toggle-mode",
        label: "Basculer entre vue unifiée et séparée",
        default: "m",
        scope: Scope::Reading,
    },
    ShortcutAction {
        id: "refresh-reading",
        label: "Actualiser (lecture)",
        default: "r",
        scope: Scope::Reading,
    },
    ShortcutAction {
        id: "stage",
        label: "Indexer le fichier ou les lignes sélectionnées",
        default: "s",
        scope: Scope::Reading,
    },
    ShortcutAction {
        id: "unstage",
        label: "Désindexer le fichier ou les lignes sélectionnées",
        default: "u",
        scope: Scope::Reading,
    },
    ShortcutAction {
        id: "apply-stage",
        label: "Appliquer la sélection (indexer)",
        default: "S",
        scope: Scope::Reading,
    },
    ShortcutAction {
        id: "apply-unstage",
        label: "Appliquer la sélection (désindexer)",
        default: "U",
        scope: Scope::Reading,
    },
    ShortcutAction {
        id: "mark",
        label: "Sélectionner le fichier courant",
        default: "x",
        scope: Scope::Reading,
    },
    ShortcutAction {
        id: "open-repository",
        label: "Ouvrir un dépôt",
        default: "Ctrl+O",
        scope: Scope::Global,
    },
    ShortcutAction {
        id: "refresh",
        label: "Actualiser",
        default: "Ctrl+R",
        scope: Scope::Global,
    },
    ShortcutAction {
        id: "palette",
        label: "Palette de commandes",
        default: "Ctrl+K",
        scope: Scope::Global,
    },
];

pub fn chord_of(event: &KeyEvent) -> Option<String> {
    let mods = event.mods;
    match &event.key {
        Key::Text(text) if !mods.cmd && !mods.ctrl && !mods.alt => {
            let mut chars = text.chars();
            let first = chars.next()?;
            (chars.next().is_none() && !first.is_whitespace() && !first.is_control())
                .then(|| text.clone())
        }
        Key::Command(c) if mods.cmd || mods.ctrl => {
            let mut chord = "Ctrl+".to_owned();
            if mods.alt {
                chord.push_str("Alt+");
            }
            if mods.shift {
                chord.push_str("Shift+");
            }
            chord.push(c.to_ascii_uppercase());
            Some(chord)
        }
        Key::Function(n) if (1..=24).contains(n) => {
            let mut chord = String::new();
            if mods.cmd || mods.ctrl {
                chord.push_str("Ctrl+");
            }
            if mods.alt {
                chord.push_str("Alt+");
            }
            if mods.shift {
                chord.push_str("Shift+");
            }
            chord.push_str(&format!("F{n}"));
            Some(chord)
        }
        _ => None,
    }
}

pub fn display(chord: &str) -> String {
    let mut chars = chord.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_uppercase() => format!("Maj+{c}"),
        _ => chord.to_owned(),
    }
}

fn is_plain(chord: &str) -> bool {
    chord.chars().count() == 1
}

fn fits(scope: Scope, chord: &str) -> bool {
    match scope {
        Scope::Reading => is_plain(chord),
        Scope::Global => chord.starts_with("Ctrl+") || chord.starts_with('F') && chord.len() > 1,
    }
}

#[derive(Default)]
pub struct ShortcutState {
    pub open: bool,
    pub custom: BTreeMap<String, String>,
    pub query: TextEdit,
    pub selected: Option<&'static str>,
    pub capturing: Option<&'static str>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub version: u32,
}

impl ShortcutState {
    pub fn chord(&self, action: &ShortcutAction) -> String {
        self.custom
            .get(action.id)
            .cloned()
            .unwrap_or_else(|| action.default.to_owned())
    }

    pub fn action_for(&self, chord: &str, scope: Scope) -> Option<&'static str> {
        ACTIONS
            .iter()
            .find(|a| a.scope == scope && self.chord(a) == chord)
            .map(|a| a.id)
    }

    pub fn validate(&self, id: &str, chord: &str) -> Result<(), String> {
        let action = ACTIONS
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| "action inconnue".to_owned())?;
        if RESERVED.contains(&chord) {
            return Err(format!(
                "{} est réservée au copier-coller et à l'édition : choisissez une autre combinaison.",
                display(chord)
            ));
        }
        if !fits(action.scope, chord) {
            return Err(match action.scope {
                Scope::Reading => "Cette action de lecture n'accepte qu'une touche seule (lettre, chiffre ou symbole), sans Ctrl : elle ne doit pas gêner la saisie.".to_owned(),
                Scope::Global => "Cette action globale demande Ctrl+une touche, ou une touche de fonction.".to_owned(),
            });
        }
        if let Some(other) = ACTIONS
            .iter()
            .find(|a| a.id != id && a.scope == action.scope && self.chord(a) == chord)
        {
            return Err(format!(
                "{} est déjà affectée à « {} » : changez-la d'abord.",
                display(chord),
                other.label
            ));
        }
        Ok(())
    }
}

impl State {
    pub fn shortcut_for(&self, id: &str) -> String {
        ACTIONS
            .iter()
            .find(|a| a.id == id)
            .map(|a| display(&self.shortcuts.chord(a)))
            .unwrap_or_default()
    }

    fn save_shortcuts(&mut self) {
        let _ = self.prefs.set(SHORTCUTS_KEY, &self.shortcuts.custom);
    }

    pub fn open_shortcuts(&mut self) {
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
        self.close_workspaces();
        self.profiles_open = false;
        let custom = std::mem::take(&mut self.shortcuts.custom);
        self.shortcuts = ShortcutState {
            open: true,
            custom,
            selected: ACTIONS.first().map(|a| a.id),
            version: self.shortcuts.version + 1,
            ..ShortcutState::default()
        };
    }

    pub fn close_shortcuts(&mut self) {
        self.shortcuts.open = false;
        self.shortcuts.capturing = None;
    }

    pub fn shortcut_visible(&self) -> Vec<&'static ShortcutAction> {
        let needle = crate::palette::fold(&self.shortcuts.query.text);
        let tokens: Vec<&str> = needle.split_whitespace().collect();
        ACTIONS
            .iter()
            .filter(|a| {
                let hay = format!(
                    "{} {}",
                    crate::palette::fold(a.label),
                    crate::palette::fold(&display(&self.shortcuts.chord(a)))
                );
                tokens.iter().all(|t| hay.contains(t))
            })
            .collect()
    }

    pub fn shortcut_select(&mut self, id: &'static str) {
        self.shortcuts.selected = Some(id);
        self.shortcuts.capturing = None;
        self.shortcuts.error = None;
        self.shortcuts.notice = None;
        self.shortcuts.version += 1;
    }

    pub fn shortcut_begin_capture(&mut self) {
        if self.shortcuts.selected.is_some() {
            self.shortcuts.capturing = self.shortcuts.selected;
            self.shortcuts.error = None;
            self.shortcuts.notice = None;
            self.shortcuts.version += 1;
        }
    }

    pub fn shortcut_capture(&mut self, event: &KeyEvent) -> bool {
        let Some(id) = self.shortcuts.capturing else {
            return false;
        };
        if event.key == Key::Escape {
            self.shortcuts.capturing = None;
            self.shortcuts.version += 1;
            return true;
        }
        let Some(chord) = chord_of(event) else {
            if !matches!(event.key, Key::Text(_) | Key::Command(_) | Key::Function(_)) {
                self.shortcuts.error = Some(
                    "Cette touche ne peut pas être affectée : utilisez une lettre, un chiffre, un symbole, Ctrl+une touche ou une touche de fonction.".to_owned(),
                );
                self.shortcuts.capturing = None;
                self.shortcuts.version += 1;
            }
            return true;
        };
        self.shortcuts.capturing = None;
        match self.shortcuts.validate(id, &chord) {
            Ok(()) => {
                let default = ACTIONS.iter().find(|a| a.id == id).map(|a| a.default);
                if default == Some(chord.as_str()) {
                    self.shortcuts.custom.remove(id);
                } else {
                    self.shortcuts.custom.insert(id.to_owned(), chord.clone());
                }
                self.save_shortcuts();
                self.shortcuts.error = None;
                self.shortcuts.notice =
                    Some(format!("Nouvelle combinaison : {}.", display(&chord)));
            }
            Err(message) => {
                self.shortcuts.error = Some(message);
                self.shortcuts.notice = None;
            }
        }
        self.shortcuts.version += 1;
        true
    }

    pub fn shortcut_reset(&mut self) {
        let Some(id) = self.shortcuts.selected else {
            return;
        };
        let Some(action) = ACTIONS.iter().find(|a| a.id == id) else {
            return;
        };
        let mut without = self.shortcuts.custom.clone();
        without.remove(id);
        let probe = ShortcutState {
            custom: without,
            ..ShortcutState::default()
        };
        match probe.validate(id, action.default) {
            Ok(()) => {
                self.shortcuts.custom.remove(id);
                self.save_shortcuts();
                self.shortcuts.error = None;
                self.shortcuts.notice = Some(format!(
                    "Combinaison par défaut rétablie : {}.",
                    display(action.default)
                ));
            }
            Err(message) => {
                self.shortcuts.error = Some(format!(
                    "Impossible de rétablir la combinaison par défaut : {message}"
                ));
                self.shortcuts.notice = None;
            }
        }
        self.shortcuts.version += 1;
    }

    pub fn shortcut_reset_all(&mut self) {
        self.shortcuts.custom.clear();
        self.save_shortcuts();
        self.shortcuts.capturing = None;
        self.shortcuts.error = None;
        self.shortcuts.notice =
            Some("Toutes les combinaisons par défaut sont rétablies.".to_owned());
        self.shortcuts.version += 1;
    }
}
