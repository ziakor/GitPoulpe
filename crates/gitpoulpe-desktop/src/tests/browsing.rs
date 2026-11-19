use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use gitpoulpe_hosting::{MemoryStore, SecretStore};
use rune::{Runtime, TextEdit};

use super::{click_at, rect_of, runtime, step};
use crate::accounts::Hosting;
use crate::state::State;

struct Fake {
    child: Child,
    port: u16,
    _dir: tempfile::TempDir,
}

impl Fake {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../gitpoulpe-hosting/tests/common/fake_github.py");
        let mut child = Command::new("python3")
            .arg("-I")
            .arg(script)
            .arg(dir.path().join("log"))
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        Self {
            child,
            port: line.trim().parse().unwrap(),
            _dir: dir,
        }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn settle(rt: &mut Runtime<State>, until: impl Fn(&State) -> bool) {
    for _ in 0..400 {
        step(rt);
        if until(&rt.state) {
            for _ in 0..8 {
                step(rt);
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the screen never settled");
}

fn click(rt: &mut Runtime<State>, key: &str) {
    if key.starts_with("ac-") && !key.starts_with("ac-row") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("ac-panel", target));
        for _ in 0..8 {
            step(rt);
        }
    }
    if key.starts_with("br-") && !key.starts_with("br-row") && !key.starts_with("br-scope") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("br-panel", target));
        for _ in 0..8 {
            step(rt);
        }
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn screen(store: Arc<MemoryStore>) -> Runtime<State> {
    let mut state = State::new(None, rune::prefs::Prefs::memory());
    state.hosting = Hosting::new(store);
    state.setup(Arc::new(|| {}));
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "profiles");
    settle(&mut rt, |s| s.profiles_open);
    click(&mut rt, "accounts");
    settle(&mut rt, |s| s.accounts.open);
    click(&mut rt, "ac-enterprise");
    settle(&mut rt, |s| s.accounts.enterprise);
    rt
}

fn connect(rt: &mut Runtime<State>, fake: &Fake, token: &str) {
    let before = rt.state.accounts.book.connections.len();
    rt.state.accounts.server = TextEdit::new(fake.base());
    rt.state.accounts.token = TextEdit::new(token);
    step(rt);
    click(rt, "ac-connect");
    settle(rt, move |s| {
        !s.accounts.busy && s.accounts.book.connections.len() > before
    });
}

fn browse(rt: &mut Runtime<State>) {
    click(rt, "ac-browse");
    settle(rt, |s| {
        s.accounts.browser.open && !s.accounts.browser.loading
    });
}

#[test]
fn the_account_repositories_and_organizations_are_listed_filtered_and_cloned_from_the_list() {
    let fake = Fake::start();
    let mut rt = screen(Arc::new(MemoryStore::default()));
    connect(&mut rt, &fake, "good");
    browse(&mut rt);
    assert!(shows(&rt, "Dépôts · Octo@"));
    for name in ["acme/Zed", "octo/a", "octo/b"] {
        assert!(shows(&rt, name), "{name}");
    }
    assert!(shows(&rt, "privé · secret"));
    assert!(shows(&rt, "fork"));
    assert!(rect_of(&rt, "br-scope-me").is_some() && rect_of(&rt, "br-scope1").is_some());
    click(&mut rt, "br-scope0");
    settle(&mut rt, |s| {
        s.accounts.browser.scope.as_deref() == Some("acme") && !s.accounts.browser.loading
    });
    assert!(shows(&rt, "acme/tool"));
    assert!(shows(&rt, "archivé · lecture seule"));
    assert!(!shows(&rt, "octo/a"));
    click(&mut rt, "br-scope-me");
    settle(&mut rt, |s| {
        s.accounts.browser.scope.is_none() && !s.accounts.browser.loading
    });
    rt.state.accounts.browser.filter = TextEdit::new("SECRET");
    rt.state.accounts.browser.version += 1;
    for _ in 0..8 {
        step(&mut rt);
    }
    assert!(shows(&rt, "octo/b") && !shows(&rt, "octo/a"));
    rt.state.accounts.browser.filter = TextEdit::new("zzz");
    rt.state.accounts.browser.version += 1;
    for _ in 0..8 {
        step(&mut rt);
    }
    assert!(shows(&rt, "Aucun dépôt ne correspond au filtre"));
    rt.state.accounts.browser.filter = TextEdit::new("");
    rt.state.accounts.browser.version += 1;
    for _ in 0..8 {
        step(&mut rt);
    }
    click(&mut rt, "brk2");
    step(&mut rt);
    assert!(shows(&rt, "https://example.test/octo/b.git"));
    click(&mut rt, "br-clone");
    settle(&mut rt, |s| s.clone.open);
    assert_eq!(rt.state.clone.url.text, "https://example.test/octo/b.git");
    assert!(!rt.state.accounts.open && !rt.state.accounts.browser.open);
}

#[test]
fn two_servers_with_the_same_repository_names_stay_apart_and_a_late_answer_never_replaces_the_current_account()
 {
    let first = Fake::start();
    let second = Fake::start();
    let mut rt = screen(Arc::new(MemoryStore::default()));
    connect(&mut rt, &first, "slow");
    connect(&mut rt, &second, "good2");
    let ids: Vec<String> = rt
        .state
        .accounts
        .book
        .connections
        .iter()
        .map(|c| c.id.clone())
        .collect();
    let slow = ids.iter().find(|i| i.ends_with("/slow")).unwrap().clone();
    let other = ids.iter().find(|i| i.ends_with("/octo2")).unwrap().clone();
    rt.state.select_account(slow.clone());
    step(&mut rt);
    click(&mut rt, "ac-browse");
    step(&mut rt);
    assert!(rt.state.accounts.browser.loading);
    click(&mut rt, "br-back");
    settle(&mut rt, |s| !s.accounts.browser.open);
    rt.state.select_account(other.clone());
    step(&mut rt);
    browse(&mut rt);
    assert!(shows(&rt, "Dépôts · Octo2@"));
    assert!(shows(&rt, "octo/a"));
    std::thread::sleep(std::time::Duration::from_secs(7));
    for _ in 0..20 {
        step(&mut rt);
    }
    assert_eq!(
        rt.state.accounts.browser.connection.as_deref(),
        Some(other.as_str())
    );
    assert!(!shows(&rt, "slow/late"));
    assert!(
        rt.state
            .accounts
            .browser
            .repos
            .iter()
            .all(|r| r.connection_id == other && r.clone_url.contains("two.test"))
    );
}

#[test]
fn a_restricted_or_revoked_token_shows_its_own_message_and_never_falls_back_to_another_server() {
    let fake = Fake::start();
    let store = Arc::new(MemoryStore::default());
    let mut rt = screen(store.clone());
    connect(&mut rt, &fake, "good");
    let key = rt.state.accounts.book.connections[0].secret_key();
    store.set(&key, "sso").unwrap();
    click(&mut rt, "ac-browse");
    settle(&mut rt, |s| s.accounts.browser.error.is_some());
    assert!(shows(&rt, "autorisation SSO"));
    assert!(shows(&rt, "Organisations indisponibles"));
    assert!(rt.state.accounts.browser.repos.is_empty());
    store.set(&key, "bad").unwrap();
    click(&mut rt, "br-refresh");
    settle(&mut rt, |s| {
        s.accounts
            .browser
            .error
            .as_deref()
            .is_some_and(|e| e.contains("expiré"))
    });
    assert!(shows(&rt, "expiré, révoqué ou invalide"));
}
