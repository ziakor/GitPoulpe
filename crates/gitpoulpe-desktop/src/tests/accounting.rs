use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use gitpoulpe_core::{Profile, SigningPreference};
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
    for _ in 0..300 {
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
    rt
}

fn connect(rt: &mut Runtime<State>, fake: &Fake, token: &str) {
    if !rt.state.accounts.enterprise {
        click(rt, "ac-enterprise");
        settle(rt, |s| s.accounts.enterprise);
    }
    rt.state.accounts.server = TextEdit::new(fake.base());
    rt.state.accounts.token = TextEdit::new(token);
    step(rt);
    click(rt, "ac-connect");
}

#[test]
fn connecting_shows_the_account_and_scopes_and_keeps_the_token_out_of_the_preferences() {
    let fake = Fake::start();
    let store = Arc::new(MemoryStore::default());
    let mut rt = screen(store.clone());
    assert!(shows(&rt, "Aucun compte connecté"));
    connect(&mut rt, &fake, "good");
    settle(&mut rt, |s| !s.accounts.busy && s.accounts.done.is_some());
    assert!(shows(&rt, "Connecté en tant que Octo"));
    settle(&mut rt, |s| {
        matches!(
            s.accounts.statuses.values().next(),
            Some(crate::accounts::Status::Ok(_))
        )
    });
    assert!(shows(&rt, "portées : repo, read:org"));
    assert_eq!(rt.state.accounts.token.text, "");
    let saved = serde_json::to_string(&rt.state.accounts.book).unwrap();
    assert!(!saved.contains("good\""), "{saved}");
    assert_eq!(store.keys().len(), 1);
    assert_eq!(rt.state.accounts.book.connections.len(), 1);
}

#[test]
fn a_revoked_token_is_reported_on_the_next_check_and_reconnecting_recovers() {
    let fake = Fake::start();
    let store = Arc::new(MemoryStore::default());
    let mut rt = screen(store.clone());
    connect(&mut rt, &fake, "good");
    settle(&mut rt, |s| !s.accounts.busy && s.accounts.done.is_some());
    let key = rt.state.accounts.book.connections[0].secret_key();
    store.set(&key, "revoked").unwrap();
    click(&mut rt, "ac-check");
    settle(&mut rt, |s| {
        matches!(
            s.accounts.statuses.values().next(),
            Some(crate::accounts::Status::Failed(_))
        )
    });
    assert!(shows(&rt, "expiré, révoqué ou invalide"));
    connect(&mut rt, &fake, "good");
    settle(&mut rt, |s| {
        !s.accounts.busy
            && matches!(
                s.accounts.statuses.values().next(),
                Some(crate::accounts::Status::Ok(_))
            )
    });
    assert_eq!(rt.state.accounts.book.connections.len(), 1);
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("good"));
}

#[test]
fn a_rejected_token_leaves_nothing_behind_and_explains_why() {
    let fake = Fake::start();
    let store = Arc::new(MemoryStore::default());
    let mut rt = screen(store.clone());
    connect(&mut rt, &fake, "sso");
    settle(&mut rt, |s| !s.accounts.busy && s.accounts.error.is_some());
    assert!(shows(&rt, "autorisation SSO"));
    assert!(store.keys().is_empty());
    assert!(rt.state.accounts.book.connections.is_empty());
    rt.state.accounts.server = TextEdit::new("http://git.example.org");
    step(&mut rt);
    click(&mut rt, "ac-connect");
    settle(&mut rt, |s| {
        s.accounts
            .error
            .as_deref()
            .is_some_and(|e| e.contains("HTTPS"))
    });
}

#[test]
fn disconnecting_removes_the_token_but_never_touches_local_git_profiles() {
    let fake = Fake::start();
    let store = Arc::new(MemoryStore::default());
    let mut rt = screen(store.clone());
    rt.state
        .book
        .upsert(Profile {
            id: "p1".into(),
            label: "Perso".into(),
            user_name: "Ada".into(),
            user_email: "ada@example.org".into(),
            signing: SigningPreference::None,
        })
        .unwrap();
    let profiles = rt.state.book.clone();
    connect(&mut rt, &fake, "good");
    settle(&mut rt, |s| !s.accounts.busy && s.accounts.done.is_some());
    click(&mut rt, "ac-disconnect");
    settle(&mut rt, |s| s.accounts.book.connections.is_empty());
    assert!(store.keys().is_empty());
    assert!(shows(&rt, "vos profils Git sont inchangés"));
    assert_eq!(rt.state.book, profiles);
}
