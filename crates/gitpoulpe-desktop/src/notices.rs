use std::collections::BTreeMap;
use std::path::PathBuf;

use gitpoulpe_core::{Notice, NoticeBook, NoticeKind, Pushed};
use gitpoulpe_git::remote_urls;
use gitpoulpe_hosting::{RawNotice, fetch_notifications, parse_remote};
use rune::TextEdit;

use crate::engine::Event;
use crate::state::State;
use crate::workspaces::{RepoHealth, health};

pub const NOTICES_KEY: &str = "notices";

pub enum Target {
    Conflicts,
    Pull { number: Option<u64> },
}

pub struct Pending {
    pub path: String,
    pub target: Target,
}

#[derive(Default)]
pub struct NoticesState {
    pub open: bool,
    pub book: NoticeBook,
    pub request: u64,
    pub fetching: usize,
    pub errors: BTreeMap<String, String>,
    pub message: Option<String>,
    pub pending: Option<Pending>,
    pub pull_number: Option<u64>,
    pub version: u32,
}

fn describe(kind: NoticeKind) -> &'static str {
    match kind {
        NoticeKind::Review => "revue demandée",
        NoticeKind::Mention => "mention",
        NoticeKind::Checks => "vérifications",
        NoticeKind::Activity => "activité",
        NoticeKind::Conflicts => "conflits",
        NoticeKind::System => "alerte",
    }
}

pub fn kind_label(kind: NoticeKind) -> &'static str {
    describe(kind)
}

impl State {
    fn save_notices(&mut self) {
        let _ = self.prefs.set(NOTICES_KEY, &self.notices.book);
    }

    pub fn notices_unread(&self) -> usize {
        self.notices.book.unread()
    }

    pub fn push_local_notice(
        &mut self,
        kind: NoticeKind,
        title: String,
        repo_path: Option<String>,
    ) {
        let at = format!("{:?}", std::time::SystemTime::now());
        let notice = Notice {
            id: format!(
                "local:{kind:?}:{}:{title}",
                repo_path.as_deref().unwrap_or("-")
            ),
            connection: None,
            repo_path,
            kind,
            title,
            repo: None,
            number: None,
            at,
            count: 1,
            read: false,
        };
        if self.notices.book.push(notice) != Pushed::Muted {
            self.save_notices();
            self.notices.version += 1;
        }
    }

    pub fn open_notices(&mut self) {
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
        self.close_shortcuts();
        self.close_workspaces();
        self.close_favorites();
        self.close_overview();
        self.accounts.open = false;
        self.profiles_open = false;
        self.notices.open = true;
        self.notices.message = None;
        self.notices.version += 1;
    }

    pub fn close_notices(&mut self) {
        self.notices.open = false;
    }

    pub fn notices_refresh(&mut self) {
        self.notices.request += 1;
        let request = self.notices.request;
        self.notices.errors.clear();
        self.notices.fetching = 0;
        if !self.notices.book.prefs.enabled {
            self.notices.message =
                Some("Les notifications sont désactivées dans les préférences.".to_owned());
            self.notices.version += 1;
            return;
        }
        let connections: Vec<_> = self
            .accounts
            .book
            .connections
            .iter()
            .filter(|c| !self.notices.book.prefs.muted_connections.contains(&c.id))
            .cloned()
            .collect();
        for connection in connections {
            self.notices.fetching += 1;
            let secrets = self.hosting.secrets.clone();
            let sender = self.event_tx();
            let waker = self.waker_slot();
            self.hosting.runtime.spawn(async move {
                let result = fetch_notifications(secrets.as_ref(), &connection)
                    .await
                    .map_err(|e| e.to_string());
                let _ = sender.send(Event::NoticesFetched {
                    request,
                    connection: connection.id,
                    result,
                });
                if let Some(wake) = waker.lock().unwrap().as_ref() {
                    wake();
                }
            });
        }
        self.notices.version += 1;
    }

    pub fn apply_notices_fetched(
        &mut self,
        request: u64,
        connection: String,
        result: Result<Vec<RawNotice>, String>,
    ) -> bool {
        if request != self.notices.request || self.accounts.book.get(&connection).is_none() {
            return false;
        }
        self.notices.fetching = self.notices.fetching.saturating_sub(1);
        match result {
            Ok(items) => {
                for raw in items {
                    let notice = Notice {
                        id: format!("{connection}:{}", raw.id),
                        connection: Some(connection.clone()),
                        repo_path: None,
                        kind: raw.kind,
                        title: raw.title,
                        repo: raw.repo,
                        number: raw.number,
                        at: raw.at,
                        count: 1,
                        read: false,
                    };
                    self.notices.book.push(notice);
                }
                self.save_notices();
            }
            Err(message) => {
                self.notices.errors.insert(connection, message);
            }
        }
        self.notices.version += 1;
        true
    }

    pub fn notices_toggle_enabled(&mut self) {
        let prefs = &mut self.notices.book.prefs;
        prefs.enabled = !prefs.enabled;
        self.save_notices();
        self.notices.version += 1;
    }

    pub fn notices_toggle_kind(&mut self, kind: NoticeKind) {
        let muted = &mut self.notices.book.prefs.muted_kinds;
        if !muted.remove(&kind) {
            muted.insert(kind);
        }
        self.save_notices();
        self.notices.version += 1;
    }

    pub fn notices_toggle_connection(&mut self, id: &str) {
        let muted = &mut self.notices.book.prefs.muted_connections;
        if !muted.remove(id) {
            muted.insert(id.to_owned());
        }
        self.save_notices();
        self.notices.version += 1;
    }

    pub fn notices_mark_all_read(&mut self) {
        self.notices.book.mark_all_read();
        self.save_notices();
        self.notices.version += 1;
    }

    pub fn notices_dismiss(&mut self, id: &str) {
        self.notices.book.dismiss(id);
        self.save_notices();
        self.notices.version += 1;
    }

    fn local_repo_for(
        &self,
        connection: &gitpoulpe_core::Connection,
        full_name: &str,
    ) -> Option<String> {
        let host = connection.host_caption();
        let wanted = full_name.to_lowercase();
        self.overview_candidates()
            .into_iter()
            .find_map(|candidate| {
                if health(&candidate.path) != RepoHealth::Ok {
                    return None;
                }
                remote_urls(std::path::Path::new(&candidate.path))
                    .iter()
                    .filter_map(|url| parse_remote(url))
                    .any(|r| {
                        r.host == host && format!("{}/{}", r.owner, r.repo).to_lowercase() == wanted
                    })
                    .then_some(candidate.path)
            })
    }

    pub fn notices_open(&mut self, id: &str) {
        let Some(notice) = self.notices.book.items.iter().find(|n| n.id == id).cloned() else {
            return;
        };
        self.notices.book.mark_read(id);
        self.save_notices();
        self.notices.message = None;
        match notice.kind {
            NoticeKind::Conflicts => {
                let Some(path) = notice.repo_path else {
                    return;
                };
                self.notices_go(path, Target::Conflicts);
            }
            NoticeKind::System => {
                self.notices.message = Some(notice.title);
                self.notices.version += 1;
            }
            _ => self.notices_open_remote(notice),
        }
    }

    fn notices_open_remote(&mut self, notice: Notice) {
        let (Some(connection_id), Some(repo)) = (notice.connection.clone(), notice.repo.clone())
        else {
            return;
        };
        let Some(connection) = self.accounts.book.get(&connection_id).cloned() else {
            self.notices.message = Some(format!(
                "Le compte qui a reçu cette notification n'est plus connecté : reconnectez-le dans « Comptes fournisseurs » pour ouvrir {repo}."
            ));
            self.notices.version += 1;
            return;
        };
        if let Some(why) = self.notices.errors.get(&connection_id) {
            self.notices.message = Some(format!(
                "L'accès de {}@{} est refusé ({why}) : reconnectez le compte avant d'ouvrir cette notification.",
                connection.login,
                connection.host_caption()
            ));
            self.notices.version += 1;
            return;
        }
        match self.local_repo_for(&connection, &repo) {
            Some(path) => self.notices_go(
                path,
                Target::Pull {
                    number: notice.number,
                },
            ),
            None => {
                let url = format!("{}/{repo}.git", connection.web);
                self.notices.message = Some(format!(
                    "{repo} n'est cloné nulle part dans vos espaces et favoris : l'écran de clonage est prêt."
                ));
                self.close_notices();
                self.begin_clone();
                self.clone.url = TextEdit::new(url);
            }
        }
    }

    fn notices_go(&mut self, path: String, target: Target) {
        let same = self.repo.as_ref().is_some_and(|r| {
            r.path.canonicalize().ok() == PathBuf::from(&path).canonicalize().ok()
        });
        self.close_notices();
        if same {
            self.notices.pending = Some(Pending { path, target });
            self.apply_notice_target();
        } else {
            self.notices.pending = Some(Pending {
                path: path.clone(),
                target,
            });
            self.open_path(PathBuf::from(path));
        }
    }

    pub fn apply_notice_target(&mut self) {
        let Some(pending) = self.notices.pending.take() else {
            return;
        };
        let Some(repo) = self.repo.as_ref().map(|r| r.path.clone()) else {
            return;
        };
        if repo.canonicalize().ok() != PathBuf::from(&pending.path).canonicalize().ok() {
            return;
        }
        match pending.target {
            Target::Conflicts => self.open_conflicts(),
            Target::Pull { number, .. } => {
                self.notices.pull_number = number;
                self.open_pulls();
            }
        }
    }
}
