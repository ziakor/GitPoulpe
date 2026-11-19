use gitpoulpe_core::{Remote, RemoteImpact};
use gitpoulpe_git::{FetchOutcome, HostKey, NetError, RemoteCheck};
use rune::TextEdit;

use crate::engine::{BranchDone, Command, RemoteOp};
use crate::state::State;

pub enum CheckStatus {
    Running,
    Reachable(RemoteCheck),
    Failed(NetError),
}

pub enum ScanStatus {
    Scanning,
    Keys(Vec<HostKey>),
    Failed(NetError),
}

pub enum FetchStatus {
    Running { phase: String, percent: u8 },
    Done(FetchOutcome),
    Failed(NetError),
}

pub struct FetchState {
    pub remote: String,
    pub status: FetchStatus,
}

pub struct CheckState {
    pub name: String,
    pub status: CheckStatus,
    pub scan: Option<ScanStatus>,
}

pub struct RemotesState {
    pub open: bool,
    pub request: u64,
    pub op_request: u64,
    pub impact_request: u64,
    pub remotes: Vec<Remote>,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub selected: Option<String>,
    pub new_name: TextEdit,
    pub new_url: TextEdit,
    pub url: TextEdit,
    pub push_url: TextEdit,
    pub rename: TextEdit,
    pub confirm_remove: Option<(String, Option<RemoteImpact>)>,
    pub check: Option<CheckState>,
    pub fetch: Option<FetchState>,
    pub fetch_request: u64,
    pub check_request: u64,
    pub scan_request: u64,
    pub version: u32,
}

impl Default for RemotesState {
    fn default() -> Self {
        Self {
            open: false,
            request: 0,
            op_request: 0,
            impact_request: 0,
            remotes: Vec::new(),
            loading: false,
            busy: false,
            error: None,
            selected: None,
            new_name: TextEdit::new(""),
            new_url: TextEdit::new(""),
            url: TextEdit::new(""),
            push_url: TextEdit::new(""),
            rename: TextEdit::new(""),
            confirm_remove: None,
            check: None,
            fetch: None,
            fetch_request: 0,
            check_request: 0,
            scan_request: 0,
            version: 0,
        }
    }
}

impl RemotesState {
    pub fn net_busy(&self) -> bool {
        matches!(
            self.check.as_ref().map(|c| &c.status),
            Some(CheckStatus::Running)
        ) || matches!(
            self.fetch.as_ref().map(|f| &f.status),
            Some(FetchStatus::Running { .. })
        )
    }

    pub fn selected_remote(&self) -> Option<&Remote> {
        let name = self.selected.as_deref()?;
        self.remotes.iter().find(|r| r.name == name)
    }

    fn clear_forms(&mut self) {
        self.url = TextEdit::new("");
        self.push_url = TextEdit::new("");
        self.rename = TextEdit::new("");
        self.confirm_remove = None;
    }
}

impl State {
    pub fn open_remotes(&mut self) {
        if self.repo.is_none() {
            return;
        }
        self.close_history();
        self.close_ignored();
        self.close_branches();
        self.close_tags();
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
        self.close_notices();
        self.close_issues();
        self.profiles_open = false;
        self.remotes.open = true;
        self.reload_remotes();
    }

    pub fn close_remotes(&mut self) {
        self.cancel_remote_check();
        self.remotes.open = false;
        self.remotes.confirm_remove = None;
    }

    pub(crate) fn reload_remotes(&mut self) {
        self.remotes.request += 1;
        self.remotes.loading = true;
        self.remotes.error = None;
        self.remotes.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::RemoteList {
                request: self.remotes.request,
            });
        }
    }

    pub fn select_remote(&mut self, name: &str) {
        self.cancel_remote_check();
        self.remotes.check = None;
        self.remotes.fetch = None;
        self.remotes.selected = Some(name.to_owned());
        self.remotes.clear_forms();
        self.remotes.version += 1;
    }

    fn send_remote_op(&mut self, op: RemoteOp) {
        if self.remotes.busy {
            return;
        }
        self.remotes.busy = true;
        self.remotes.op_request += 1;
        self.remotes.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::RemoteOp {
                request: self.remotes.op_request,
                op,
            });
        }
    }

    pub fn remote_add(&mut self) {
        let name = self.remotes.new_name.text.trim().to_owned();
        let url = self.remotes.new_url.text.trim().to_owned();
        if name.is_empty() || url.is_empty() {
            self.notice = Some("Donnez un nom et une URL au remote.".to_owned());
            return;
        }
        self.send_remote_op(RemoteOp::Add { name, url });
    }

    pub fn remote_set_url(&mut self) {
        let Some(remote) = self.remotes.selected_remote() else {
            return;
        };
        let url = self.remotes.url.text.trim().to_owned();
        if url.is_empty() {
            self.notice = Some("Saisissez la nouvelle URL de lecture.".to_owned());
            return;
        }
        let (name, expected) = (remote.name.clone(), remote.fetch_urls.clone());
        self.send_remote_op(RemoteOp::SetUrl {
            name,
            url,
            expected,
        });
    }

    pub fn remote_set_push_url(&mut self, clear: bool) {
        let Some(remote) = self.remotes.selected_remote() else {
            return;
        };
        let typed = self.remotes.push_url.text.trim().to_owned();
        if !clear && typed.is_empty() {
            self.notice = Some("Saisissez la nouvelle URL d'envoi.".to_owned());
            return;
        }
        let (name, expected) = (remote.name.clone(), remote.push_urls.clone());
        self.send_remote_op(RemoteOp::SetPushUrl {
            name,
            url: (!clear).then_some(typed),
            expected,
        });
    }

    pub fn remote_rename(&mut self) {
        let Some(remote) = self.remotes.selected_remote() else {
            return;
        };
        let new = self.remotes.rename.text.trim().to_owned();
        if new.is_empty() {
            self.notice = Some("Saisissez le nouveau nom.".to_owned());
            return;
        }
        let old = remote.name.clone();
        self.send_remote_op(RemoteOp::Rename { old, new });
    }

    pub fn remote_ask_remove(&mut self) {
        let Some(remote) = self.remotes.selected_remote() else {
            return;
        };
        let name = remote.name.clone();
        self.remotes.confirm_remove = Some((name.clone(), None));
        self.remotes.impact_request += 1;
        self.remotes.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::RemoteImpact {
                request: self.remotes.impact_request,
                name,
            });
        }
    }

    pub fn remote_cancel_remove(&mut self) {
        self.remotes.confirm_remove = None;
        self.remotes.version += 1;
    }

    pub fn remote_remove(&mut self) {
        let Some((name, Some(_))) = self.remotes.confirm_remove.take() else {
            return;
        };
        let Some(remote) = self.remotes.remotes.iter().find(|r| r.name == name) else {
            return;
        };
        let expected = remote.fetch_urls.clone();
        self.send_remote_op(RemoteOp::Remove { name, expected });
    }

    pub fn apply_remotes_listed(
        &mut self,
        request: u64,
        result: Result<Vec<Remote>, String>,
    ) -> bool {
        if request != self.remotes.request {
            return false;
        }
        self.remotes.loading = false;
        match result {
            Ok(list) => {
                if self
                    .remotes
                    .selected
                    .as_ref()
                    .is_some_and(|n| !list.iter().any(|r| &r.name == n))
                {
                    self.remotes.selected = None;
                }
                self.remotes.remotes = list;
                self.remotes.error = None;
                self.pulls_remotes_ready();
            }
            Err(message) => self.remotes.error = Some(message),
        }
        self.remotes.version += 1;
        true
    }

    pub fn apply_remote_impacted(
        &mut self,
        request: u64,
        name: String,
        result: Result<RemoteImpact, String>,
    ) -> bool {
        if request != self.remotes.impact_request {
            return false;
        }
        match (&mut self.remotes.confirm_remove, result) {
            (Some((pending, slot)), Ok(impact)) if *pending == name => *slot = Some(impact),
            (_, Err(message)) => {
                self.remotes.confirm_remove = None;
                self.notice = Some(format!("Impact du retrait illisible : {message}"));
            }
            _ => return false,
        }
        self.remotes.version += 1;
        true
    }

    pub fn apply_remote_done(&mut self, request: u64, result: Result<BranchDone, String>) -> bool {
        if request != self.remotes.op_request {
            return false;
        }
        self.remotes.busy = false;
        self.remotes.confirm_remove = None;
        match result {
            Ok(done) => {
                if let Some((action, before, after, detail)) = done.journal {
                    self.record_journal(&action, before, after, detail);
                }
                self.notice = Some(done.message);
                self.remotes.new_name = TextEdit::new("");
                self.remotes.new_url = TextEdit::new("");
                self.remotes.clear_forms();
                self.reload_remotes();
                self.request_refresh(false);
            }
            Err(message) => {
                self.notice = Some(format!("Remote non modifié : {message}"));
                self.reload_remotes();
            }
        }
        self.remotes.version += 1;
        true
    }

    pub fn remote_check(&mut self) {
        let Some(remote) = self.remotes.selected_remote() else {
            return;
        };
        let name = remote.name.clone();
        if self.remotes.net_busy() {
            return;
        }
        self.remotes.check_request += 1;
        self.remotes.check = Some(CheckState {
            name: name.clone(),
            status: CheckStatus::Running,
            scan: None,
        });
        self.remotes.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::RemoteCheck {
                request: self.remotes.check_request,
                name,
            });
        }
    }

    pub fn cancel_remote_check(&mut self) {
        if self.remotes.net_busy()
            && let Some(engine) = &self.engine
        {
            engine.send(Command::NetCancel);
        }
    }

    fn check_host(&self) -> Option<(String, Option<u16>)> {
        let check = self.remotes.check.as_ref()?;
        let remote = self.remotes.remotes.iter().find(|r| r.name == check.name)?;
        gitpoulpe_git::remote_host(remote.fetch_urls.first()?)
    }

    pub fn remote_scan_host(&mut self) {
        let Some((host, port)) = self.check_host() else {
            return;
        };
        let Some(check) = self.remotes.check.as_mut() else {
            return;
        };
        check.scan = Some(ScanStatus::Scanning);
        self.remotes.scan_request += 1;
        self.remotes.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::SshScan {
                request: self.remotes.scan_request,
                host,
                port,
            });
        }
    }

    pub fn remote_trust_host(&mut self, index: usize) {
        let Some((host, port)) = self.check_host() else {
            return;
        };
        let Some(CheckState {
            scan: Some(ScanStatus::Keys(keys)),
            ..
        }) = &self.remotes.check
        else {
            return;
        };
        let Some(key) = keys.get(index) else {
            return;
        };
        let fingerprint = key.fingerprint.clone();
        self.remotes.scan_request += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::SshTrust {
                request: self.remotes.scan_request,
                host,
                port,
                fingerprint,
            });
        }
    }

    pub fn apply_remote_checked(
        &mut self,
        request: u64,
        name: String,
        result: Result<RemoteCheck, NetError>,
    ) -> bool {
        if request != self.remotes.check_request {
            return false;
        }
        match &mut self.remotes.check {
            Some(check) if check.name == name => {
                check.status = match result {
                    Ok(done) => CheckStatus::Reachable(done),
                    Err(error) => CheckStatus::Failed(error),
                };
            }
            _ => return false,
        }
        self.remotes.version += 1;
        true
    }

    pub fn apply_ssh_scanned(
        &mut self,
        request: u64,
        result: Result<Vec<HostKey>, NetError>,
    ) -> bool {
        if request != self.remotes.scan_request {
            return false;
        }
        let Some(check) = self.remotes.check.as_mut() else {
            return false;
        };
        check.scan = Some(match result {
            Ok(keys) => ScanStatus::Keys(keys),
            Err(error) => ScanStatus::Failed(error),
        });
        self.remotes.version += 1;
        true
    }

    pub fn apply_ssh_trusted(&mut self, request: u64, result: Result<(), NetError>) -> bool {
        if request != self.remotes.scan_request {
            return false;
        }
        match result {
            Ok(()) => {
                self.notice = Some("Hôte enregistré dans known_hosts.".to_owned());
                self.remote_check();
            }
            Err(error) => {
                if let Some(check) = self.remotes.check.as_mut() {
                    check.scan = Some(ScanStatus::Failed(error));
                }
            }
        }
        self.remotes.version += 1;
        true
    }

    pub fn remote_fetch(&mut self) {
        let Some(remote) = self.remotes.selected_remote() else {
            return;
        };
        if self.remotes.net_busy() {
            return;
        }
        let name = remote.name.clone();
        self.remotes.fetch_request += 1;
        self.remotes.check = None;
        self.remotes.fetch = Some(FetchState {
            remote: name.clone(),
            status: FetchStatus::Running {
                phase: "Connexion".to_owned(),
                percent: 0,
            },
        });
        self.remotes.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::Fetch {
                request: self.remotes.fetch_request,
                remote: name,
            });
        }
    }

    pub fn apply_fetch_progress(&mut self, request: u64, phase: String, percent: u8) -> bool {
        if request != self.remotes.fetch_request {
            return false;
        }
        match &mut self.remotes.fetch {
            Some(FetchState {
                status:
                    FetchStatus::Running {
                        phase: p,
                        percent: c,
                    },
                ..
            }) => {
                *p = phase;
                *c = percent;
            }
            _ => return false,
        }
        self.remotes.version += 1;
        true
    }

    pub fn apply_fetch_done(
        &mut self,
        request: u64,
        result: Result<FetchOutcome, NetError>,
    ) -> bool {
        if request != self.remotes.fetch_request {
            return false;
        }
        let Some(state) = self.remotes.fetch.as_mut() else {
            return false;
        };
        match result {
            Ok(outcome) => {
                state.status = FetchStatus::Done(outcome);
                self.request_refresh(false);
            }
            Err(error) => state.status = FetchStatus::Failed(error),
        }
        self.remotes.version += 1;
        true
    }
}
