use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use gitpoulpe_core::{Connection, Provider};
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
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repo(origin: &str, branch: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["remote", "add", "origin", origin]);
    std::fs::write(p.join("a.txt"), "a\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    if branch != "main" {
        git(p, &["checkout", "-q", "-b", branch]);
    }
    dir
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
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the screen never settled");
}

fn click(rt: &mut Runtime<State>, key: &str) {
    if key.starts_with("pr-") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("pr-panel", target));
        for _ in 0..8 {
            step(rt);
        }
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn connection(fake: &Fake, login: &str) -> Connection {
    Connection {
        id: format!("127.0.0.1:{}/{}", fake.port, login.to_lowercase()),
        provider: Provider::Github,
        api: fake.base(),
        web: fake.base(),
        login: login.to_owned(),
        scopes: Vec::new(),
    }
}

fn screen(dir: &Path, fake: &Fake, logins: &[&str]) -> Runtime<State> {
    let store = Arc::new(MemoryStore::default());
    let mut state = State::new(None, rune::prefs::Prefs::memory());
    for login in logins {
        let connection = connection(fake, login);
        store.set(&connection.secret_key(), "good").unwrap();
        state.accounts.book.upsert(connection);
    }
    state.hosting = Hosting::new(store);
    state.engine_git = Some(gitpoulpe_git::Git::new().with_env("GIT_CONFIG_GLOBAL", "/dev/null"));
    state.setup(Arc::new(|| {}));
    state.open_blocking(dir.to_path_buf(), Duration::from_secs(20));
    let mut rt = runtime(state);
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    click(&mut rt, "branches-pulls");
    settle(&mut rt, |s| s.pulls.open && !s.pulls.loading);
    rt
}

#[test]
fn the_form_shows_branches_and_the_diff_then_creates_a_draft_once() {
    let fake = Fake::start();
    let dir = repo(&format!("{}/octo/proj.git", fake.base()), "feature/x");
    let mut rt = screen(dir.path(), &fake, &["Octo"]);
    assert!(shows(&rt, "octo/proj:feature/x → octo/proj:main"));
    assert!(shows(&rt, "2 commit(s) d'avance, 1 de retard"));
    assert!(shows(&rt, "modified src/a.rs (+3 −1)"));
    assert!(shows(&rt, "Compte : Octo@"));
    assert_eq!(rt.state.pulls.title.text, "X");
    rt.state.pulls.title = TextEdit::new("Add the thing");
    step(&mut rt);
    click(&mut rt, "pr-draft");
    step(&mut rt);
    click(&mut rt, "pr-create");
    settle(&mut rt, |s| !s.pulls.busy && s.pulls.created.is_some());
    assert!(shows(&rt, "Pull request #12 créée (brouillon)"));
    assert!(rect_of(&rt, "pr-create").is_none());
    assert!(shows(&rt, "déjà ouverte"));
    rt.state.create_pull();
    assert!(!rt.state.pulls.busy);
}

#[test]
fn an_unpublished_branch_and_an_existing_pull_offer_an_explicit_way_forward_and_no_create_button() {
    let fake = Fake::start();
    let dir = repo(&format!("{}/octo/proj.git", fake.base()), "local-only");
    let rt = screen(dir.path(), &fake, &["Octo"]);
    assert!(shows(&rt, "La branche local-only n'est pas publiée"));
    assert!(rect_of(&rt, "pr-create").is_none());
    let dup = repo(&format!("{}/octo/proj.git", fake.base()), "feature/dup");
    let rt = screen(dup.path(), &fake, &["Octo"]);
    assert!(shows(&rt, "Une pull request est déjà ouverte : #7"));
    assert!(rect_of(&rt, "pr-create").is_none());
}

#[test]
fn a_remote_without_a_matching_account_or_a_detached_head_explains_what_to_do() {
    let fake = Fake::start();
    let dir = repo("https://example.org/octo/proj.git", "feature/x");
    let rt = screen(dir.path(), &fake, &["Octo"]);
    assert!(shows(&rt, "Aucun compte connecté ne correspond"));
    assert!(rt.state.pulls.target.is_none());
    let local = repo("/srv/git/proj.git", "feature/x");
    let rt = screen(local.path(), &fake, &["Octo"]);
    assert!(shows(&rt, "Aucun compte connecté ne correspond"));
    let detached = repo(&format!("{}/octo/proj.git", fake.base()), "main");
    git(detached.path(), &["checkout", "-q", "--detach"]);
    let rt = screen(detached.path(), &fake, &["Octo"]);
    assert!(shows(&rt, "HEAD est détachée"));
}

#[test]
fn from_a_fork_the_parent_can_be_targeted_and_two_matching_accounts_can_be_switched() {
    let fake = Fake::start();
    let dir = repo(&format!("{}/octo/forked.git", fake.base()), "feature/x");
    let mut rt = screen(dir.path(), &fake, &["Octo", "Octo2"]);
    assert!(shows(&rt, "Compte : Octo@"));
    assert!(shows(&rt, "octo/forked:feature/x → octo/forked:main"));
    click(&mut rt, "pr-parent");
    settle(&mut rt, |s| {
        !s.pulls.loading
            && s.pulls
                .target
                .as_ref()
                .is_some_and(|t| t.target_repo.full_name() == "acme/proj")
    });
    assert!(shows(&rt, "octo/forked:feature/x → acme/proj:main"));
    click(&mut rt, "pr-account-next");
    settle(&mut rt, |s| !s.pulls.loading && s.pulls.target.is_some());
    assert!(shows(&rt, "Compte : Octo2@"));
    assert!(shows(&rt, "octo/forked:feature/x → octo/forked:main"));
    assert!(!rt.state.pulls.to_parent);
}

#[test]
fn origin_is_preferred_over_other_matching_remotes() {
    let fake = Fake::start();
    let dir = repo(&format!("{}/octo/forked.git", fake.base()), "feature/x");
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "aaa",
            &format!("{}/octo/proj.git", fake.base()),
        ],
    );
    let rt = screen(dir.path(), &fake, &["Octo"]);
    assert_eq!(rt.state.pulls.candidates[0].remote, "origin");
    assert!(shows(&rt, "octo/forked:feature/x → octo/forked:main"));
}

#[test]
fn an_answer_arriving_after_the_screen_was_left_creates_no_notice() {
    let fake = Fake::start();
    let dir = repo(&format!("{}/octo/proj.git", fake.base()), "feature/x");
    let mut rt = screen(dir.path(), &fake, &["Octo"]);
    rt.state.create_pull();
    assert!(rt.state.pulls.busy);
    rt.state.pulls.open = false;
    rt.state.pulls.request += 1;
    rt.state.pulls.open = true;
    for _ in 0..60 {
        step(&mut rt);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(rt.state.pulls.created.is_none() && rt.state.pulls.notice.is_none());
}
