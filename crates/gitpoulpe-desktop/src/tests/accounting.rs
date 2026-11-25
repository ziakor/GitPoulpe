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
    log: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl Fake {
    fn start() -> Self {
        Self::start_script("fake_github.py")
    }

    fn start_script(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../gitpoulpe-hosting/tests/common")
            .join(name);
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
            log: dir.path().join("log"),
            _dir: dir,
        }
    }

    fn polls(&self) -> usize {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .matches("login/oauth/access_token")
            .count()
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
    super::menu(&mut rt, "nav-accounts");
    settle(&mut rt, |s| s.accounts.open);
    rt
}

fn manual(rt: &mut Runtime<State>) {
    if !rt.state.accounts.manual {
        click(rt, "ac-other");
        settle(rt, |s| s.accounts.manual);
    }
}

fn card_of(rt: &Runtime<State>, provider: gitpoulpe_core::Provider) -> usize {
    rt.state
        .accounts
        .book
        .connections
        .iter()
        .position(|c| c.provider == provider)
        .expect("a card for this provider")
}

fn connect(rt: &mut Runtime<State>, fake: &Fake, token: &str) {
    manual(rt);
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
    click(&mut rt, "ac-check0");
    settle(&mut rt, |s| {
        matches!(
            s.accounts.statuses.values().next(),
            Some(crate::accounts::Status::Failed(_))
        )
    });
    assert!(shows(&rt, "expiré, révoqué ou invalide") && shows(&rt, "À reconnecter"));
    rt.state.accounts.manual = false;
    rt.state.accounts.enterprise = false;
    rt.state.accounts.server = TextEdit::new("");
    step(&mut rt);
    click(&mut rt, "ac-reconnect0");
    settle(&mut rt, |s| s.accounts.manual);
    assert!(
        rt.state.accounts.enterprise && rt.state.accounts.server.text == fake.base(),
        "the form comes back filled in for this server"
    );
    rt.state.accounts.token = TextEdit::new("good");
    step(&mut rt);
    click(&mut rt, "ac-connect");
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
    click(&mut rt, "ac-disconnect0");
    settle(&mut rt, |s| s.accounts.book.connections.is_empty());
    assert!(store.keys().is_empty());
    assert!(shows(&rt, "vos profils Git sont inchangés"));
    assert_eq!(rt.state.book, profiles);
}

fn point_oauth_at(rt: &mut Runtime<State>, fake: &Fake, second_ms: u64) {
    rt.state.accounts.oauth_host = gitpoulpe_hosting::GithubHost {
        api: fake.base(),
        web: fake.base(),
    };
    rt.state.accounts.oauth_client_id = "client-1".to_owned();
    rt.state.accounts.second_ms = second_ms;
    step(rt);
}

#[test]
fn browser_sign_in_connects_the_account_without_pasting_a_token() {
    let fake = Fake::start();
    let store = Arc::new(MemoryStore::default());
    let mut rt = screen(store.clone());
    point_oauth_at(&mut rt, &fake, 1);
    assert!(rect_of(&rt, "ac-device").is_some());
    click(&mut rt, "ac-device");
    settle(&mut rt, |s| !s.accounts.busy && s.accounts.done.is_some());
    assert!(shows(&rt, "Connecté en tant que Octo"));
    assert!(rt.state.accounts.device.is_none());
    let key = rt.state.accounts.book.connections[0].secret_key();
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("good"));
    assert_eq!(rt.state.accounts.token.text, "");
}

#[test]
fn the_code_is_copied_and_the_verification_page_opened_in_one_click() {
    let fake = Fake::start();
    let store = Arc::new(MemoryStore::default());
    let mut rt = screen(store.clone());
    point_oauth_at(&mut rt, &fake, 60_000);
    click(&mut rt, "ac-device");
    settle(&mut rt, |s| s.accounts.device.is_some());
    let issued = rt.state.accounts.device.clone().unwrap();
    let clipboard = rune::clipboard::memory();
    rt.use_clipboard(&clipboard);
    rt.take_url_requests();
    assert!(shows(&rt, "Copier le code"));
    click(&mut rt, "ac-device-open");
    step(&mut rt);
    assert_eq!(
        clipboard.text().map(|t| t.to_string()).as_deref(),
        Some(issued.user_code.as_str())
    );
    assert_eq!(
        rt.take_url_requests(),
        std::slice::from_ref(&issued.verification_uri)
    );
    assert!(shows(&rt, "Code copié"));
    click(&mut rt, "ac-device-cancel");
    settle(&mut rt, |s| s.accounts.device.is_none() && !s.accounts.busy);
    assert!(store.keys().is_empty());
}

#[test]
fn the_code_is_shown_while_waiting_and_cancelling_stops_everything_and_stores_nothing() {
    let fake = Fake::start();
    let store = Arc::new(MemoryStore::default());
    let mut rt = screen(store.clone());
    point_oauth_at(&mut rt, &fake, 600);
    click(&mut rt, "ac-device");
    settle(&mut rt, |s| s.accounts.device.is_some());
    let issued = rt.state.accounts.device.clone().unwrap();
    let request = rt.state.accounts.request;
    assert!(shows(&rt, "ABCD-1234"));
    assert!(shows(&rt, "/login/device"));
    assert!(shows(&rt, "En attente de votre autorisation"));
    assert!(rect_of(&rt, "ac-device").is_none());
    click(&mut rt, "ac-device-cancel");
    settle(&mut rt, |s| s.accounts.device.is_none() && !s.accounts.busy);
    assert!(!shows(&rt, "ABCD-1234"));
    assert!(rect_of(&rt, "ac-device").is_some());
    assert!(rt.state.accounts.device_task.is_none());
    assert!(!rt.state.apply_account_device_code(request, issued));
    assert!(rt.state.accounts.device.is_none());
    let polls_at_cancel = fake.polls();
    std::thread::sleep(std::time::Duration::from_secs(7));
    step(&mut rt);
    assert_eq!(fake.polls(), polls_at_cancel);
    assert!(store.keys().is_empty());
    assert!(rt.state.accounts.book.connections.is_empty());
}

#[test]
fn a_wrong_client_id_is_explained_and_browser_sign_in_ignores_the_manual_form() {
    let fake = Fake::start();
    let mut rt = screen(Arc::new(MemoryStore::default()));
    point_oauth_at(&mut rt, &fake, 1);
    rt.state.accounts.oauth_client_id = "other".to_owned();
    assert!(
        rect_of(&rt, "ac-token").is_none(),
        "the token form starts folded"
    );
    click(&mut rt, "ac-device");
    settle(&mut rt, |s| !s.accounts.busy && s.accounts.error.is_some());
    assert!(shows(&rt, "identifiant d'application OAuth inconnu"));
    manual(&mut rt);
    click(&mut rt, "ac-prov-gitlab");
    click(&mut rt, "ac-enterprise");
    settle(&mut rt, |s| s.accounts.enterprise);
    rt.state.accounts.oauth_client_id = "client-1".to_owned();
    click(&mut rt, "ac-device");
    settle(&mut rt, |s| !s.accounts.busy && s.accounts.done.is_some());
    let connection = &rt.state.accounts.book.connections[0];
    assert_eq!(
        (connection.provider, connection.login.as_str()),
        (gitpoulpe_core::Provider::Github, "Octo"),
        "browser sign-in goes to GitHub whatever the token form is set to"
    );
    assert!(!rt.state.accounts.manual, "the form folds once connected");
}

fn connect_gitlab_account(rt: &mut Runtime<State>, fake: &Fake, token: &str) {
    manual(rt);
    click(rt, "ac-prov-gitlab");
    settle(rt, |s| {
        s.accounts.provider == gitpoulpe_core::Provider::Gitlab
    });
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
fn a_gitlab_account_connects_next_to_a_github_one_with_its_scopes_and_expiry_and_disconnects_alone()
{
    let github = Fake::start();
    let gitlab = Fake::start_script("fake_gitlab.py");
    let store = Arc::new(MemoryStore::default());
    let mut rt = screen(store.clone());
    connect(&mut rt, &github, "good");
    settle(&mut rt, |s| !s.accounts.busy && s.accounts.done.is_some());
    connect_gitlab_account(&mut rt, &gitlab, "glgood");
    settle(&mut rt, |s| {
        !s.accounts.busy && s.accounts.book.connections.len() == 2
    });
    assert!(shows(&rt, "Connecté en tant que Octo sur 127.0.0.1"));
    assert!(rect_of(&rt, "ac-row0").is_some() && rect_of(&rt, "ac-row1").is_some());
    settle(&mut rt, |s| {
        s.accounts
            .statuses
            .values()
            .filter(|st| matches!(st, crate::accounts::Status::Ok(_)))
            .count()
            == 2
    });
    assert!(shows(
        &rt,
        "portées : read_api, read_repository, write_repository · expire le 2027-01-31"
    ));
    assert!(shows(&rt, "GitLab") && shows(&rt, "GitHub"));
    let mut keys = store.keys();
    keys.sort();
    assert!(
        keys[0].starts_with("github:") && keys[1].starts_with("gitlab:"),
        "{keys:?}"
    );
    let gitlab_id = rt
        .state
        .accounts
        .book
        .connections
        .iter()
        .find(|c| c.provider == gitpoulpe_core::Provider::Gitlab)
        .unwrap()
        .id
        .clone();
    assert!(rt.state.accounts.book.get(&gitlab_id).is_some());
    let card = card_of(&rt, gitpoulpe_core::Provider::Gitlab);
    click(&mut rt, &format!("ac-disconnect{card}"));
    settle(&mut rt, |s| s.accounts.book.connections.len() == 1);
    assert_eq!(store.keys().len(), 1);
    assert!(store.keys()[0].starts_with("github:"));
}

#[test]
fn a_rejected_gitlab_token_leaves_nothing_behind_and_explains_why() {
    let gitlab = Fake::start_script("fake_gitlab.py");
    let store = Arc::new(MemoryStore::default());
    let mut rt = screen(store.clone());
    connect_gitlab_account(&mut rt, &gitlab, "gone");
    settle(&mut rt, |s| !s.accounts.busy && s.accounts.error.is_some());
    assert!(shows(&rt, "expiré, révoqué ou invalide"));
    assert!(store.keys().is_empty() && rt.state.accounts.book.connections.is_empty());
}

#[test]
fn reconnecting_a_github_com_account_shows_the_code_inside_its_own_card() {
    let fake = Fake::start();
    let mut rt = screen(Arc::new(MemoryStore::default()));
    point_oauth_at(&mut rt, &fake, 600);
    let connection = gitpoulpe_core::Connection {
        id: "github.com/octo".to_owned(),
        provider: gitpoulpe_core::Provider::Github,
        api: fake.base(),
        web: "https://github.com".to_owned(),
        login: "Octo".to_owned(),
        scopes: Vec::new(),
    };
    rt.state.accounts.book.upsert(connection.clone());
    rt.state.accounts.statuses.insert(
        connection.id.clone(),
        crate::accounts::Status::Failed("le jeton est expiré".to_owned()),
    );
    rt.state.accounts.version += 1;
    step(&mut rt);
    click(&mut rt, "ac-reconnect0");
    settle(&mut rt, |s| s.accounts.device.is_some());
    let card = rect_of(&rt, "ac-row0").unwrap();
    let code = rect_of(&rt, "ac-device-code").unwrap();
    assert!(
        code[1] > card[1] && code[1] + code[3] <= card[1] + card[3],
        "the code sits in the card: {card:?} {code:?}"
    );
    assert!(
        rect_of(&rt, "ac-device").is_some(),
        "adding another account stays possible"
    );
    assert!(rect_of(&rt, "ac-reconnect0").is_none());
    click(&mut rt, "ac-device-cancel");
    settle(&mut rt, |s| s.accounts.device.is_none() && !s.accounts.busy);
    assert!(rt.state.accounts.device_for.is_none());
    assert!(rect_of(&rt, "ac-reconnect0").is_some());
}
