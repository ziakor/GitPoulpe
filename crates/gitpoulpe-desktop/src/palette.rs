use rune::TextEdit;
use rune::{Key, Tx};

use crate::state::{Screen, State, ViewKind};

pub struct PaletteCommand {
    pub id: &'static str,
    pub label: &'static str,
    pub keywords: &'static str,
    pub available: fn(&State) -> bool,
    pub run: fn(&mut State, &mut Tx),
}

#[derive(Default)]
pub struct PaletteState {
    pub open: bool,
    pub query: TextEdit,
    pub selected: Option<&'static str>,
    pub restore_focus: Option<String>,
    pub version: u32,
}

fn always(_: &State) -> bool {
    true
}

fn in_repo(s: &State) -> bool {
    s.repo.is_some() && s.screen == Screen::Ready
}

fn current_path(s: &State) -> Option<String> {
    let (_, entry) = s.current.and_then(|p| s.entry_at(p))?;
    entry.path.to_str().map(str::to_owned)
}

fn on_file(s: &State) -> bool {
    in_repo(s) && current_path(s).is_some()
}

fn on_unstaged_file(s: &State) -> bool {
    in_repo(s)
        && s.view == ViewKind::Unstaged
        && s.current
            .and_then(|p| s.entry_at(p))
            .is_some_and(|(_, e)| e.is_unstaged() && !e.is_conflict())
}

fn on_staged_file(s: &State) -> bool {
    in_repo(s)
        && s.view == ViewKind::Staged
        && s.current
            .and_then(|p| s.entry_at(p))
            .is_some_and(|(_, e)| e.is_staged())
}

fn with_operation(s: &State) -> bool {
    in_repo(s) && s.operation_kind.is_some()
}

pub const COMMANDS: &[PaletteCommand] = &[
    PaletteCommand {
        id: "open-repository",
        label: "Ouvrir un dépôt…",
        keywords: "dossier ouvrir repository",
        available: always,
        run: |_, tx| crate::keys::open_dialog(tx),
    },
    PaletteCommand {
        id: "clone",
        label: "Cloner un dépôt",
        keywords: "telecharger url clone",
        available: always,
        run: |s, _| s.begin_clone(),
    },
    PaletteCommand {
        id: "accounts",
        label: "Comptes fournisseurs",
        keywords: "github gitlab connexion jeton token",
        available: always,
        run: |s, _| s.open_accounts(),
    },
    PaletteCommand {
        id: "profiles",
        label: "Profils Git",
        keywords: "identite nom email signature",
        available: always,
        run: |s, _| s.open_profiles(),
    },
    PaletteCommand {
        id: "tools",
        label: "Outils externes…",
        keywords: "editeur diff fusion terminal shell configuration",
        available: always,
        run: |s, _| s.open_tools_config(),
    },
    PaletteCommand {
        id: "shortcuts",
        label: "Raccourcis clavier…",
        keywords: "touches clavier personnaliser combinaison",
        available: always,
        run: |s, _| s.open_shortcuts(),
    },
    PaletteCommand {
        id: "favorites",
        label: "Favoris…",
        keywords: "depots favoris marque-pages recherche",
        available: always,
        run: |s, _| s.open_favorites(),
    },
    PaletteCommand {
        id: "favorite-current",
        label: "Ajouter le dépôt courant aux favoris",
        keywords: "favori etoile marquer",
        available: in_repo,
        run: |s, _| s.favorites_add_current(),
    },
    PaletteCommand {
        id: "workspaces",
        label: "Espaces de travail…",
        keywords: "groupe depots projets restaurer",
        available: always,
        run: |s, _| s.open_workspaces(),
    },
    PaletteCommand {
        id: "theme",
        label: "Changer de thème",
        keywords: "clair sombre dark light",
        available: always,
        run: |s, _| s.set_dark(!s.dark),
    },
    PaletteCommand {
        id: "refresh",
        label: "Actualiser le dépôt",
        keywords: "recharger statut refresh",
        available: in_repo,
        run: |s, _| s.refresh(),
    },
    PaletteCommand {
        id: "history",
        label: "Historique des commits",
        keywords: "log graphe commits",
        available: in_repo,
        run: |s, _| s.open_history(),
    },
    PaletteCommand {
        id: "branches",
        label: "Branches",
        keywords: "branche checkout creer supprimer",
        available: in_repo,
        run: |s, _| s.open_branches(),
    },
    PaletteCommand {
        id: "tags",
        label: "Étiquettes (tags)",
        keywords: "tag version release",
        available: in_repo,
        run: |s, _| s.open_tags(None),
    },
    PaletteCommand {
        id: "remotes",
        label: "Dépôts distants",
        keywords: "remote origin fetch push pull",
        available: in_repo,
        run: |s, _| s.open_remotes(),
    },
    PaletteCommand {
        id: "stashes",
        label: "Remisages (stash)",
        keywords: "stash remiser",
        available: in_repo,
        run: |s, _| s.open_stashes(),
    },
    PaletteCommand {
        id: "reflog",
        label: "Reflog",
        keywords: "journal reference recuperer",
        available: in_repo,
        run: |s, _| s.open_reflog(),
    },
    PaletteCommand {
        id: "worktrees",
        label: "Worktrees et terminal",
        keywords: "arbre de travail terminal shell",
        available: in_repo,
        run: |s, _| s.open_worktrees(),
    },
    PaletteCommand {
        id: "hooks",
        label: "Hooks Git",
        keywords: "crochets pre-commit",
        available: in_repo,
        run: |s, _| s.open_hooks(),
    },
    PaletteCommand {
        id: "lfs",
        label: "Git LFS",
        keywords: "gros fichiers large",
        available: in_repo,
        run: |s, _| s.open_lfs(),
    },
    PaletteCommand {
        id: "submodules",
        label: "Sous-modules",
        keywords: "submodule",
        available: in_repo,
        run: |s, _| s.open_submodules(),
    },
    PaletteCommand {
        id: "gitflow",
        label: "Gitflow",
        keywords: "feature release hotfix flux",
        available: in_repo,
        run: |s, _| s.open_gitflow(),
    },
    PaletteCommand {
        id: "pulls",
        label: "Pull requests et merge requests",
        keywords: "pr mr revue fusion",
        available: in_repo,
        run: |s, _| s.open_pulls(),
    },
    PaletteCommand {
        id: "ignored",
        label: "Fichiers ignorés",
        keywords: "gitignore exclure",
        available: in_repo,
        run: |s, _| s.open_ignored(None),
    },
    PaletteCommand {
        id: "undo",
        label: "Annuler ou rétablir une opération",
        keywords: "undo redo journal",
        available: in_repo,
        run: |s, _| s.open_undo(),
    },
    PaletteCommand {
        id: "conflicts",
        label: "Résoudre les conflits",
        keywords: "fusion merge rebase conflit",
        available: with_operation,
        run: |s, _| s.open_conflicts(),
    },
    PaletteCommand {
        id: "toggle-mode",
        label: "Basculer entre vue unifiée et séparée",
        keywords: "diff unifie separe split",
        available: in_repo,
        run: |s, tx| s.toggle_mode(tx),
    },
    PaletteCommand {
        id: "file-history",
        label: "Historique du fichier courant",
        keywords: "fichier blame renommage",
        available: on_file,
        run: |s, _| {
            if let Some(path) = current_path(s) {
                s.open_file_history(path);
            }
        },
    },
    PaletteCommand {
        id: "file-edit",
        label: "Éditer le fichier courant",
        keywords: "modifier editeur fichier",
        available: on_unstaged_file,
        run: |s, _| {
            if let Some(path) = current_path(s) {
                s.open_file_edit(path);
            }
        },
    },
    PaletteCommand {
        id: "stage-current",
        label: "Indexer le fichier courant",
        keywords: "stage add fichier",
        available: on_unstaged_file,
        run: |s, _| s.stage_current(),
    },
    PaletteCommand {
        id: "unstage-current",
        label: "Désindexer le fichier courant",
        keywords: "unstage reset fichier",
        available: on_staged_file,
        run: |s, _| s.unstage_current(),
    },
];

pub fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .flat_map(|c| match c {
            'é' | 'è' | 'ê' | 'ë' => vec!['e'],
            'à' | 'â' | 'ä' => vec!['a'],
            'î' | 'ï' => vec!['i'],
            'ô' | 'ö' => vec!['o'],
            'ù' | 'û' | 'ü' => vec!['u'],
            'ç' => vec!['c'],
            'œ' => vec!['o', 'e'],
            other => vec![other],
        })
        .collect()
}

pub fn matching<'a>(
    state: &State,
    commands: &'a [PaletteCommand],
    query: &str,
) -> Vec<&'a PaletteCommand> {
    let tokens: Vec<String> = fold(query).split_whitespace().map(str::to_owned).collect();
    let mut found: Vec<(usize, &PaletteCommand)> = commands
        .iter()
        .filter(|c| (c.available)(state))
        .filter_map(|c| {
            let label = fold(c.label);
            let haystack = format!("{label} {}", fold(c.keywords));
            if !tokens.iter().all(|t| haystack.contains(t.as_str())) {
                return None;
            }
            let rank = tokens
                .iter()
                .map(|t| label.find(t.as_str()).unwrap_or(1000))
                .sum();
            Some((rank, c))
        })
        .collect();
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, c)| c).collect()
}

impl State {
    pub fn palette_results(&self) -> Vec<&'static PaletteCommand> {
        matching(self, COMMANDS, &self.commands.query.text)
    }

    pub fn palette_selected_id(&self) -> Option<&'static str> {
        let results = self.palette_results();
        self.commands
            .selected
            .filter(|id| results.iter().any(|c| c.id == *id))
            .or_else(|| results.first().map(|c| c.id))
    }

    pub fn open_palette(&mut self, tx: &mut Tx) {
        if self.commands.open {
            return;
        }
        let restore = tx.focused().map(str::to_owned);
        self.commands = PaletteState {
            open: true,
            restore_focus: restore,
            version: self.commands.version + 1,
            ..PaletteState::default()
        };
        self.commands.selected = self.palette_results().first().map(|c| c.id);
        tx.focus("pal-input".to_owned(), true);
    }

    pub fn close_palette(&mut self, tx: &mut Tx, restore: bool) {
        if !self.commands.open {
            return;
        }
        self.commands.open = false;
        self.commands.version += 1;
        let previous = self.commands.restore_focus.take();
        match (restore, previous) {
            (true, Some(key)) => tx.focus(key, true),
            _ => tx.blur(),
        }
    }

    pub fn palette_move(&mut self, delta: i32, tx: &mut Tx) {
        let results = self.palette_results();
        if results.is_empty() {
            return;
        }
        let current = self
            .palette_selected_id()
            .and_then(|id| results.iter().position(|c| c.id == id))
            .unwrap_or(0) as i32;
        let next = (current + delta).rem_euclid(results.len() as i32) as usize;
        self.commands.selected = Some(results[next].id);
        self.commands.version += 1;
        tx.scroll_into_view_in("pal-list", format!("pal-row{next}"));
    }

    pub fn palette_run(&mut self, id: Option<&'static str>, tx: &mut Tx) {
        let Some(id) = id.or_else(|| self.palette_selected_id()) else {
            return;
        };
        let Some(command) = self.palette_results().into_iter().find(|c| c.id == id) else {
            return;
        };
        self.close_palette(tx, false);
        (command.run)(self, tx);
    }

    pub fn palette_key(&mut self, key: &Key, tx: &mut Tx) -> bool {
        match key {
            Key::Escape => self.close_palette(tx, true),
            Key::Down => self.palette_move(1, tx),
            Key::Up => self.palette_move(-1, tx),
            Key::Enter => self.palette_run(None, tx),
            _ => {
                self.commands.selected = None;
                self.commands.version += 1;
                return false;
            }
        }
        true
    }
}
