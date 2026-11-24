use gitpoulpe_core::Favorite;
use rune::{Key, TextEdit, Tx};

use crate::overview::Candidate;
use crate::palette::fold;
use crate::state::State;
use crate::workspaces::{RepoHealth, health};

#[derive(Default)]
pub struct SwitcherState {
    pub open: bool,
    pub query: TextEdit,
    pub candidates: Vec<Candidate>,
    pub selected: Option<String>,
    pub restore_focus: Option<String>,
    pub notice: Option<String>,
    pub version: u32,
}

fn current_path(s: &State) -> Option<String> {
    let path = &s.repo.as_ref()?.path;
    let path = path.canonicalize().unwrap_or_else(|_| path.clone());
    path.to_str().map(str::to_owned)
}

impl State {
    pub fn switcher_results(&self) -> Vec<&Candidate> {
        let tokens: Vec<String> = fold(&self.switcher.query.text)
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        self.switcher
            .candidates
            .iter()
            .filter(|c| {
                let haystack = fold(&format!("{} {}", c.name, c.path));
                tokens.iter().all(|t| haystack.contains(t.as_str()))
            })
            .collect()
    }

    pub fn switcher_selected(&self) -> Option<String> {
        let results = self.switcher_results();
        self.switcher
            .selected
            .clone()
            .filter(|path| results.iter().any(|c| &c.path == path))
            .or_else(|| results.first().map(|c| c.path.clone()))
    }

    pub fn current_is_favorite(&self) -> bool {
        let Some(current) = current_path(self) else {
            return false;
        };
        self.favorites
            .book
            .items
            .iter()
            .any(|f| matches!(f, Favorite::Local { path, .. } if *path == current))
    }

    pub(crate) fn load_switcher(&mut self) {
        let current = current_path(self);
        self.switcher.candidates = self
            .overview_candidates()
            .into_iter()
            .filter(|c| Some(&c.path) != current.as_ref())
            .collect();
        self.switcher
            .candidates
            .sort_by_cached_key(|c| (fold(&c.name), c.path.clone()));
        self.switcher.version += 1;
    }

    pub fn open_switcher(&mut self, tx: &mut Tx) {
        if self.switcher.open {
            return;
        }
        self.close_palette(tx, false);
        let restore = tx.focused().map(str::to_owned);
        self.switcher = SwitcherState {
            open: true,
            restore_focus: restore,
            version: self.switcher.version + 1,
            ..SwitcherState::default()
        };
        self.load_switcher();
        tx.focus("sw-input".to_owned(), true);
    }

    pub fn close_switcher(&mut self, tx: &mut Tx, restore: bool) {
        if !self.switcher.open {
            return;
        }
        self.switcher.open = false;
        self.switcher.version += 1;
        match (restore, self.switcher.restore_focus.take()) {
            (true, Some(key)) => tx.focus(key, true),
            _ => tx.blur(),
        }
    }

    pub fn toggle_switcher(&mut self, tx: &mut Tx) {
        if self.switcher.open {
            self.close_switcher(tx, true);
        } else {
            self.open_switcher(tx);
        }
    }

    fn switcher_move(&mut self, delta: isize) {
        let paths: Vec<String> = self
            .switcher_results()
            .iter()
            .map(|c| c.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        let at = self
            .switcher_selected()
            .and_then(|p| paths.iter().position(|q| *q == p))
            .unwrap_or(0) as isize;
        let next = (at + delta).rem_euclid(paths.len() as isize) as usize;
        self.switcher.selected = Some(paths[next].clone());
        self.switcher.version += 1;
    }

    pub fn switch_to(&mut self, path: Option<String>, tx: &mut Tx) {
        let Some(path) = path.or_else(|| self.switcher_selected()) else {
            return;
        };
        if health(&path) != RepoHealth::Ok {
            self.switcher.notice = Some(format!(
                "{path} est introuvable ou n'est plus un dépôt : corrigez-le dans les espaces de travail ou les favoris."
            ));
            self.switcher.version += 1;
            return;
        }
        self.close_switcher(tx, false);
        self.close_screens();
        self.overview_open(&path);
    }

    pub fn switcher_add_favorite(&mut self) {
        self.favorites_add_current();
        self.switcher.notice = self
            .favorites
            .error
            .clone()
            .or_else(|| self.favorites.notice.clone());
        self.switcher.version += 1;
    }

    pub fn switcher_key(&mut self, key: &Key, tx: &mut Tx) -> bool {
        match key {
            Key::Escape => self.close_switcher(tx, true),
            Key::Down => self.switcher_move(1),
            Key::Up => self.switcher_move(-1),
            Key::Enter => self.switch_to(None, tx),
            _ => {
                self.switcher.selected = None;
                self.switcher.version += 1;
                return false;
            }
        }
        true
    }
}
