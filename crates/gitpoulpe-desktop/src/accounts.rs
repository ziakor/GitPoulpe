use std::collections::HashMap;
use std::sync::Arc;

use gitpoulpe_core::{Connection, ConnectionBook};
use gitpoulpe_hosting::{
    DeviceCode, GITHUB_CLIENT_ID, GithubHost, SecretStore, Session, await_device_token,
    check_connection, connect_github, disconnect, start_device_flow,
};
use rune::TextEdit;

use crate::engine::Event;
use crate::state::State;

pub const CONNECTIONS_KEY: &str = "connections";

#[derive(Clone, Debug)]
pub enum Status {
    Checking,
    Ok(Session),
    Failed(String),
}

pub struct AccountsState {
    pub open: bool,
    pub book: ConnectionBook,
    pub request: u64,
    pub busy: bool,
    pub error: Option<String>,
    pub done: Option<String>,
    pub selected: Option<String>,
    pub statuses: HashMap<String, Status>,
    pub enterprise: bool,
    pub server: TextEdit,
    pub token: TextEdit,
    pub version: u32,
    pub device: Option<DeviceCode>,
    pub device_task: Option<tokio::task::AbortHandle>,
    pub oauth_client_id: String,
    pub oauth_host: GithubHost,
    pub second_ms: u64,
}

impl AccountsState {
    pub fn new(book: ConnectionBook) -> Self {
        Self {
            open: false,
            book,
            request: 0,
            busy: false,
            error: None,
            done: None,
            selected: None,
            statuses: HashMap::new(),
            enterprise: false,
            server: TextEdit::new(""),
            token: TextEdit::new(""),
            version: 0,
            device: None,
            device_task: None,
            oauth_client_id: GITHUB_CLIENT_ID.to_owned(),
            oauth_host: GithubHost::dotcom(),
            second_ms: 1000,
        }
    }
}

pub struct Hosting {
    pub runtime: tokio::runtime::Runtime,
    pub secrets: Arc<dyn SecretStore>,
}

impl Hosting {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("gitpoulpe-hosting")
            .enable_all()
            .build()
            .expect("hosting runtime");
        Self { runtime, secrets }
    }
}

impl State {
    pub fn open_accounts(&mut self) {
        self.profiles_open = false;
        self.accounts.open = true;
        self.accounts.error = None;
        self.accounts.done = None;
        self.accounts.version += 1;
        let all: Vec<Connection> = self.accounts.book.connections.clone();
        for connection in all {
            self.check_account(&connection.id);
        }
    }

    pub fn close_accounts(&mut self) {
        self.cancel_device();
        self.accounts.open = false;
        self.accounts.token = TextEdit::new("");
    }

    pub fn select_account(&mut self, id: String) {
        self.accounts.selected = Some(id);
        self.accounts.version += 1;
    }

    pub fn set_enterprise(&mut self, on: bool) {
        self.accounts.enterprise = on;
        self.accounts.version += 1;
    }

    pub fn check_account(&mut self, id: &str) {
        let Some(connection) = self.accounts.book.get(id).cloned() else {
            return;
        };
        self.accounts
            .statuses
            .insert(connection.id.clone(), Status::Checking);
        self.accounts.version += 1;
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = check_connection(secrets.as_ref(), &connection)
                .await
                .map_err(|e| e.to_string());
            let _ = sender.send(Event::AccountChecked {
                id: connection.id,
                result,
            });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn connect_account(&mut self) {
        if self.accounts.busy {
            return;
        }
        let token = self.accounts.token.text.clone();
        let host = if self.accounts.enterprise {
            match GithubHost::enterprise(self.accounts.server.text.trim()) {
                Ok(host) => host,
                Err(e) => {
                    self.accounts.error = Some(e.to_string());
                    self.accounts.version += 1;
                    return;
                }
            }
        } else {
            GithubHost::dotcom()
        };
        self.accounts.request += 1;
        self.accounts.busy = true;
        self.accounts.error = None;
        self.accounts.done = None;
        self.accounts.version += 1;
        let request = self.accounts.request;
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = connect_github(secrets.as_ref(), &host, &token)
                .await
                .map_err(|e| e.to_string());
            let _ = sender.send(Event::AccountConnected { request, result });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn connect_device(&mut self) {
        if self.accounts.busy || self.accounts.enterprise {
            return;
        }
        self.accounts.request += 1;
        self.accounts.busy = true;
        self.accounts.error = None;
        self.accounts.done = None;
        self.accounts.device = None;
        self.accounts.version += 1;
        let request = self.accounts.request;
        let host = self.accounts.oauth_host.clone();
        let client_id = self.accounts.oauth_client_id.clone();
        let second_ms = self.accounts.second_ms;
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        let notify = move |event: Event| {
            let _ = sender.send(event);
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        };
        let task = self.hosting.runtime.spawn(async move {
            let code = match start_device_flow(&host, &client_id).await {
                Ok(code) => code,
                Err(e) => {
                    notify(Event::AccountConnected {
                        request,
                        result: Err(e.to_string()),
                    });
                    return;
                }
            };
            notify(Event::AccountDeviceCode {
                request,
                code: code.clone(),
            });
            let result = async {
                let token = await_device_token(&host, &client_id, &code, |d| {
                    tokio::time::sleep(std::time::Duration::from_millis(d.as_secs() * second_ms))
                })
                .await?;
                connect_github(secrets.as_ref(), &host, token.expose()).await
            }
            .await
            .map_err(|e| e.to_string());
            notify(Event::AccountConnected { request, result });
        });
        self.accounts.device_task = Some(task.abort_handle());
    }

    pub fn cancel_device(&mut self) {
        if let Some(task) = self.accounts.device_task.take() {
            task.abort();
        }
        if self.accounts.device.take().is_some() || self.accounts.busy {
            self.accounts.request += 1;
            self.accounts.busy = false;
        }
        self.accounts.version += 1;
    }

    pub fn apply_account_device_code(&mut self, request: u64, code: DeviceCode) -> bool {
        if request != self.accounts.request {
            return false;
        }
        self.accounts.device = Some(code);
        self.accounts.version += 1;
        true
    }

    pub fn disconnect_account(&mut self) {
        let Some(id) = self.accounts.selected.clone() else {
            return;
        };
        let Some(connection) = self.accounts.book.get(&id).cloned() else {
            return;
        };
        match disconnect(self.hosting.secrets.as_ref(), &connection) {
            Ok(()) => {
                self.accounts.book.remove(&id);
                self.accounts.statuses.remove(&id);
                self.accounts.selected = None;
                let _ = self.prefs.set(CONNECTIONS_KEY, &self.accounts.book);
                self.accounts.done = Some(format!(
                    "{} déconnecté : le jeton est retiré, vos profils Git sont inchangés.",
                    connection.login
                ));
                self.accounts.error = None;
            }
            Err(e) => self.accounts.error = Some(e.to_string()),
        }
        self.accounts.version += 1;
    }

    pub fn apply_account_connected(
        &mut self,
        request: u64,
        result: Result<(Connection, Session), String>,
    ) -> bool {
        if request != self.accounts.request {
            return false;
        }
        self.accounts.busy = false;
        self.accounts.device = None;
        self.accounts.device_task = None;
        match result {
            Ok((connection, session)) => {
                self.accounts.done = Some(format!(
                    "Connecté en tant que {} sur {}.",
                    connection.login,
                    connection.host_caption()
                ));
                self.accounts
                    .statuses
                    .insert(connection.id.clone(), Status::Ok(session));
                self.accounts.selected = Some(connection.id.clone());
                self.accounts.book.upsert(connection);
                let _ = self.prefs.set(CONNECTIONS_KEY, &self.accounts.book);
                self.accounts.token = TextEdit::new("");
            }
            Err(message) => self.accounts.error = Some(message),
        }
        self.accounts.version += 1;
        true
    }

    pub fn apply_account_checked(&mut self, id: String, result: Result<Session, String>) -> bool {
        if self.accounts.book.get(&id).is_none() {
            return false;
        }
        self.accounts.statuses.insert(
            id,
            match result {
                Ok(session) => Status::Ok(session),
                Err(message) => Status::Failed(message),
            },
        );
        self.accounts.version += 1;
        true
    }
}
