use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use gitpoulpe_core::{Connection, Favorite, FavoriteBook, Provider};
use gitpoulpe_hosting::{MemoryStore, RepoSummary, SecretStore};
use rune::{Runtime, TextEdit};

use super::requesting::Fake;
use super::{click_at, rect_of, runtime, step};
use crate::accounts::{Hosting, Status};
use crate::state::State;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}");
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn wait_for(rt: &mut Runtime<State>, what: &str, until: impl Fn(&State) -> bool) {
    for _ in 0..500 {
        step(rt);
        if until(&rt.state) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("never saw {what}");
}

fn repo(name: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix(name).tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    dir
}

fn app(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    rt
}

fn summary(connection: &str, owner: &str, name: &str) -> RepoSummary {
    RepoSummary {
        connection_id: connection.to_owned(),
        owner: owner.to_owned(),
        name: name.to_owned(),
        description: None,
        private: false,
        fork: false,
        archived: false,
        default_branch: Some("main".to_owned()),
        clone_url: format!("https://example.test/{owner}/{name}.git"),
        parent: None,
        can_push: true,
    }
}

fn restarted(saved: &FavoriteBook) -> Runtime<State> {
    let mut prefs = rune::prefs::Prefs::memory();
    prefs.set(crate::favorites::FAVORITES_KEY, saved).unwrap();
    let mut state = State::new(None, prefs);
    state.setup(Arc::new(|| {}));
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    rt
}

#[test]
fn a_local_and_a_remote_favorite_are_found_again_after_a_restart() {
    let dir = repo("fav-proj-");
    let mut rt = app(dir.path());
    let command = crate::palette::COMMANDS
        .iter()
        .find(|c| c.id == "favorite-current")
        .unwrap();
    assert!((command.available)(&rt.state));
    rt.state.favorites_add_current();
    rt.state
        .favorites_add_remote(&summary("gh/octo", "acme", "Éclair"));
    assert_eq!(rt.state.favorites.book.items.len(), 2);
    let saved = rt.state.favorites.book.clone();
    let mut rt = restarted(&saved);
    rt.state.open_favorites();
    step(&mut rt);
    assert!(shows(&rt, "Favoris · 2"));
    rt.state.favorites.query = TextEdit::new("eclair");
    step(&mut rt);
    let found: Vec<String> = rt
        .state
        .favorites_visible()
        .iter()
        .map(|f| f.label().to_owned())
        .collect();
    assert_eq!(found, ["acme/Éclair"]);
    let name = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_lowercase();
    rt.state.favorites.query = TextEdit::new(&name[..8]);
    assert_eq!(rt.state.favorites_visible().len(), 1);
    rt.state.favorites.query = TextEdit::new("zzz");
    step(&mut rt);
    assert!(shows(&rt, "Aucun favori ne correspond"));
}

#[test]
fn local_references_and_provider_access_are_told_apart() {
    let dir = repo("fav-a-");
    let mut rt = app(dir.path());
    rt.state.favorites_add_current();
    rt.state
        .favorites_add_remote(&summary("gh/octo", "acme", "tool"));
    rt.state.open_favorites();
    step(&mut rt);
    assert!(shows(&rt, "référence locale") && shows(&rt, "accès fournisseur"));
    assert!(shows(&rt, "accessible"));
    assert!(shows(&rt, "compte déconnecté"));
}

#[test]
fn removing_a_favorite_deletes_neither_the_repository_nor_the_connection() {
    let dir = repo("fav-a-");
    let mut rt = app(dir.path());
    let connection = Connection {
        id: "gh/octo".to_owned(),
        provider: Provider::Github,
        api: "https://api.github.com".to_owned(),
        web: "https://github.com".to_owned(),
        login: "octo".to_owned(),
        scopes: Vec::new(),
    };
    rt.state.accounts.book.upsert(connection);
    rt.state.favorites_add_current();
    rt.state
        .favorites_add_remote(&summary("gh/octo", "acme", "tool"));
    rt.state.open_favorites();
    step(&mut rt);
    click(&mut rt, "fv-remove0");
    step(&mut rt);
    click(&mut rt, "fv-remove0");
    step(&mut rt);
    assert!(rt.state.favorites.book.items.is_empty());
    assert!(shows(&rt, "le dépôt et la connexion ne sont pas touchés"));
    assert!(dir.path().join(".git").exists() && dir.path().join("a.txt").exists());
    assert!(rt.state.accounts.book.get("gh/octo").is_some());
}

#[test]
fn a_moved_repository_is_flagged_refused_with_a_reason_and_repairable() {
    let dir = repo("fav-a-");
    let mut rt = app(dir.path());
    rt.state.favorites_add_current();
    let key = rt.state.favorites.book.items[0].key();
    let moved = tempfile::tempdir().unwrap();
    let new_place = moved.path().join("relocated");
    std::fs::rename(dir.path(), &new_place).unwrap();
    std::mem::forget(dir);
    rt.state.open_favorites();
    for _ in 0..30 {
        step(&mut rt);
    }
    assert!(shows(&rt, "introuvable"));
    click(&mut rt, "fv-open0");
    step(&mut rt);
    assert!(shows(&rt, "utilisez « Réparer… »"));
    assert!(rt.state.favorites.open);
    rt.state
        .favorite_repair_path(&key, moved.path().to_path_buf());
    step(&mut rt);
    assert!(shows(&rt, "n'est pas un dépôt Git"));
    rt.state.favorite_repair_path(&key, new_place.clone());
    step(&mut rt);
    assert!(shows(&rt, "Favori réparé"));
    let repaired = rt.state.favorites.book.items[0].key();
    click(&mut rt, "fv-open0");
    wait_for(&mut rt, "the repaired repository", |s| {
        !s.favorites.open
            && s.repo
                .as_ref()
                .is_some_and(|r| r.path.ends_with("relocated"))
    });
    assert!(repaired.ends_with("relocated"));
}

#[test]
fn a_revoked_or_disconnected_account_gives_a_repairable_state_and_a_working_one_clones() {
    let fake = Fake::start_script("fake_github.py");
    let dir = repo("fav-a-");
    let mut rt = app(dir.path());
    let store = Arc::new(MemoryStore::default());
    let connection = Connection {
        id: format!("127.0.0.1:{}/octo", fake.port),
        provider: Provider::Github,
        api: fake.base(),
        web: fake.base(),
        login: "Octo".to_owned(),
        scopes: Vec::new(),
    };
    store.set(&connection.secret_key(), "bad").unwrap();
    rt.state.accounts.book.upsert(connection.clone());
    rt.state.hosting = Hosting::new(store.clone());
    rt.state
        .favorites_add_remote(&summary(&connection.id, "acme", "tool"));
    rt.state.open_favorites();
    step(&mut rt);
    assert!(shows(&rt, "non vérifié"));
    click(&mut rt, "fv-check0");
    wait_for(&mut rt, "the verdict", |s| {
        matches!(
            s.accounts.statuses.get(&connection.id),
            Some(Status::Failed(_))
        )
    });
    for _ in 0..20 {
        step(&mut rt);
    }
    assert!(shows(&rt, "refusé"));
    click(&mut rt, "fv-open0");
    step(&mut rt);
    assert!(shows(
        &rt,
        "reconnectez le compte dans « Comptes fournisseurs »"
    ));
    assert!(rt.state.favorites.open && !rt.state.clone.open);
    store.set(&connection.secret_key(), "good").unwrap();
    click(&mut rt, "fv-check0");
    wait_for(&mut rt, "the new verdict", |s| {
        matches!(s.accounts.statuses.get(&connection.id), Some(Status::Ok(_)))
    });
    click(&mut rt, "fv-open0");
    step(&mut rt);
    assert!(rt.state.clone.open && !rt.state.favorites.open);
    assert_eq!(
        rt.state.clone.url.text,
        "https://example.test/acme/tool.git"
    );
    rt.state.accounts.book.remove(&connection.id);
    rt.state.open_favorites();
    step(&mut rt);
    click(&mut rt, "fv-open0");
    step(&mut rt);
    assert!(shows(&rt, "n'est plus connecté"));
}

#[test]
fn the_browser_offers_to_favorite_the_selected_repository_and_twice_is_refused() {
    let dir = repo("fav-a-");
    let mut rt = app(dir.path());
    let repo = summary("gh/octo", "acme", "tool");
    rt.state.accounts.browser.repos = vec![repo.clone()];
    rt.state.accounts.browser.selected = Some(repo.full_name());
    rt.state.favorite_selected_repo();
    assert_eq!(rt.state.favorites.book.items.len(), 1);
    rt.state.favorite_selected_repo();
    assert_eq!(rt.state.favorites.book.items.len(), 1);
    assert!(
        rt.state
            .favorites
            .error
            .as_deref()
            .is_some_and(|e| e.contains("déjà dans les favoris"))
    );
    assert!(
        matches!(&rt.state.favorites.book.items[0], Favorite::Remote { clone_url, .. } if clone_url.ends_with("acme/tool.git"))
    );
}
