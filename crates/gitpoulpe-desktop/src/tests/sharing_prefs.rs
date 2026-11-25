use std::path::Path;
use std::sync::Arc;

use gitpoulpe_core::{Connection, ConnectionBook, Provider};
use gitpoulpe_hosting::{MemoryStore, Session};
use serde_json::{Map, Value};

use crate::accounts::{CONNECTIONS_KEY, Hosting};
use crate::prefs_migration::open_at;
use crate::state::State;

fn window(path: &Path) -> State {
    let (prefs, _) = open_at(path);
    let mut state = State::new(None, prefs);
    state.hosting = Hosting::new(Arc::new(MemoryStore::default()));
    state
}

fn account(login: &str) -> (Connection, Session) {
    let connection = Connection {
        id: format!("github.com/{login}"),
        provider: Provider::Github,
        api: "https://api.github.com".to_owned(),
        web: "https://github.com".to_owned(),
        login: login.to_owned(),
        scopes: Vec::new(),
    };
    let session = Session {
        login: login.to_owned(),
        name: None,
        scopes: Vec::new(),
        fine_grained: false,
        expires_at: None,
        renews_itself: true,
    };
    (connection, session)
}

fn connect(state: &mut State, login: &str) {
    state.accounts.request += 1;
    let request = state.accounts.request;
    assert!(state.apply_account_connected(request, Ok(account(login))));
}

fn on_disk(path: &Path) -> Map<String, Value> {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn logins(path: &Path) -> Vec<String> {
    let book: ConnectionBook =
        serde_json::from_value(on_disk(path)[CONNECTIONS_KEY].clone()).unwrap();
    let mut logins: Vec<String> = book.connections.into_iter().map(|c| c.login).collect();
    logins.sort();
    logins
}

#[test]
fn two_windows_sharing_the_preferences_never_erase_what_the_other_saved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefs.json");
    let mut older = window(&path);
    let mut newer = window(&path);

    connect(&mut newer, "octo");
    older.list_width = 333.0;
    older.save_list_width();
    assert_eq!(
        logins(&path),
        ["octo"],
        "a setting saved elsewhere keeps the account"
    );
    assert_eq!(on_disk(&path)["list_width"], 333.0);

    connect(&mut older, "bot");
    assert_eq!(
        logins(&path),
        ["bot", "octo"],
        "both windows' accounts are kept"
    );
    older.disconnect_account("github.com/octo");
    assert_eq!(logins(&path), ["bot"], "only the account asked for goes");

    newer.open_accounts();
    let shown: Vec<&str> = newer
        .accounts
        .book
        .connections
        .iter()
        .map(|c| c.login.as_str())
        .collect();
    assert_eq!(
        shown,
        ["bot"],
        "opening the screen shows what the other window did"
    );
}

#[test]
fn a_file_from_a_newer_version_is_left_alone_and_a_broken_one_is_rewritten_whole() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefs.json");
    let mut state = window(&path);
    connect(&mut state, "octo");

    std::fs::write(&path, br#"{"format": 99, "kept": true}"#).unwrap();
    state.list_width = 300.0;
    state.save_list_width();
    connect(&mut state, "bot");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        r#"{"format": 99, "kept": true}"#
    );
    assert!(
        state
            .prefs_warning
            .as_deref()
            .is_some_and(|w| w.contains("version plus récente")),
        "{:?}",
        state.prefs_warning
    );
    assert_eq!(
        state.accounts.book.connections.len(),
        2,
        "the session still works"
    );

    std::fs::write(&path, b"{ not json").unwrap();
    state.save_list_width();
    assert_eq!(on_disk(&path)["list_width"], 300.0);
    assert_eq!(logins(&path), ["octo"], "the last saved state comes back");
}
