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

pub(super) struct Fake {
    child: Child,
    pub(super) port: u16,
    _dir: tempfile::TempDir,
}

impl Fake {
    fn start() -> Self {
        Self::start_script("fake_github.py")
    }

    pub(super) fn start_script(name: &str) -> Self {
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
            _dir: dir,
        }
    }

    pub(super) fn base(&self) -> String {
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

pub(super) fn repo(origin: &str, branch: &str) -> tempfile::TempDir {
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

pub(super) fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

pub(super) fn settle(rt: &mut Runtime<State>, until: impl Fn(&State) -> bool) {
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

pub(super) fn text_of(rt: &Runtime<State>, key: &str) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .and_then(|e| e.text)
}

pub(super) fn click(rt: &mut Runtime<State>, key: &str) {
    let container = if key.starts_with("prv-") || key.starts_with("mrg-") || key.starts_with("chk-")
    {
        Some("prl-side")
    } else if key.starts_with("prc-") || key.starts_with("prl-f") || key.starts_with("prl-body") {
        Some("prl-panel")
    } else {
        None
    };
    if let Some(container) = container {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in(container, target));
        for _ in 0..8 {
            step(rt);
        }
    }
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

pub(super) fn screen(dir: &Path, fake: &Fake, logins: &[&str]) -> Runtime<State> {
    screen_as(dir, fake, logins, connection)
}

pub(super) fn screen_as(
    dir: &Path,
    fake: &Fake,
    logins: &[&str],
    make: fn(&Fake, &str) -> Connection,
) -> Runtime<State> {
    let store = Arc::new(MemoryStore::default());
    let mut state = State::new(None, rune::prefs::Prefs::memory());
    for login in logins {
        let connection = make(fake, login);
        let token = if connection.provider == Provider::Gitlab {
            "glgood"
        } else {
            "good"
        };
        store.set(&connection.secret_key(), token).unwrap();
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

fn list_screen(fake: &Fake) -> (tempfile::TempDir, Runtime<State>) {
    let dir = repo(&format!("{}/acme/proj.git", fake.base()), "feature/x");
    let mut rt = screen(dir.path(), fake, &["Octo"]);
    click(&mut rt, "pr-list");
    settle(&mut rt, |s| s.pulls.list.open && !s.pulls.list.loading);
    (dir, rt)
}

pub(super) fn loaded(rt: &mut Runtime<State>, number: u64) {
    settle(rt, move |s| {
        !s.pulls.list.detail_loading
            && s.pulls
                .list
                .detail
                .as_ref()
                .is_some_and(|d| d.summary.number == number)
    });
}

#[test]
fn pull_requests_are_listed_by_state_and_one_shows_its_revisions_commits_and_diffs() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    assert!(shows(&rt, "#1 · ouverte · author1") && shows(&rt, "#4 · brouillon · author4"));
    assert!(!shows(&rt, "#2 · "));
    click(&mut rt, "prlk0");
    loaded(&mut rt, 1);
    assert_eq!(text_of(&rt, "prl-title").as_deref(), Some("PR 1"));
    assert!(shows(&rt, "feature/1 (sha1) → main (basesha)"));
    assert!(shows(&rt, "par author1"));
    assert!(shows(&rt, "Commits 2") && shows(&rt, "Fichiers 3"));
    click(&mut rt, "prl-tab-files");
    step(&mut rt);
    assert!(shows(&rt, "3 fichiers · +6 −3"));
    assert!(shows(&rt, "new3") && shows(&rt, "old1"));
    click(&mut rt, "prl-tab-commits");
    step(&mut rt);
    assert!(rect_of(&rt, "prl-commit0").is_some() && rect_of(&rt, "prl-commit1").is_some());
    assert!(shows(&rt, "first") && shows(&rt, "authorx"));
    click(&mut rt, "prl-all");
    settle(&mut rt, |s| {
        s.pulls.list.filter == gitpoulpe_hosting::PullFilter::All
            && !s.pulls.list.loading
            && s.pulls.list.items.len() == 6
    });
    assert!(shows(&rt, "fusionnée · author3"));
    assert!(shows(&rt, "fermée · author2"));
    assert!(shows(&rt, "brouillon · author4"));
}

#[test]
fn switching_pull_requests_never_shows_a_late_answer_for_the_previous_one() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    rt.state.select_pull(5);
    rt.state.select_pull(1);
    loaded(&mut rt, 1);
    std::thread::sleep(Duration::from_secs(5));
    for _ in 0..30 {
        step(&mut rt);
    }
    let detail = rt.state.pulls.list.detail.as_ref().unwrap();
    assert_eq!(detail.summary.number, 1);
    let meta = text_of(&rt, "prl-meta").unwrap();
    assert!(meta.starts_with("#1 · ") && !meta.contains("#5"));
}

#[test]
fn an_update_during_reading_and_a_removed_access_are_reported_without_mixing_results() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    rt.state.select_pull(4);
    loaded(&mut rt, 4);
    assert!(shows(&rt, "a changé pendant le chargement"));
    rt.state.select_pull(1);
    loaded(&mut rt, 1);
    assert!(rt.state.pulls.list.notice.is_none());
    rt.state.select_pull(4);
    assert!(rt.state.pulls.list.detail.is_none() && rt.state.pulls.list.notice.is_none());
    loaded(&mut rt, 4);
    rt.state.select_pull(1);
    loaded(&mut rt, 1);
    rt.state
        .pulls
        .list
        .detail
        .as_mut()
        .unwrap()
        .summary
        .head_sha = "old1234567".to_owned();
    rt.state.load_pull_detail();
    settle(&mut rt, |s| {
        !s.pulls.list.detail_loading && s.pulls.list.notice.is_some()
    });
    assert!(shows(
        &rt,
        "mise à jour depuis votre lecture (old1234 → sha1)"
    ));
    rt.state.select_pull(6);
    settle(&mut rt, |s| {
        !s.pulls.list.detail_loading && s.pulls.list.detail_error.is_some()
    });
    assert!(shows(&rt, "introuvable ou non autorisée"));
    assert!(rt.state.pulls.list.detail.is_none());
    assert!(text_of(&rt, "prl-meta").is_none());
}

fn comments_ready(rt: &mut Runtime<State>) {
    settle(rt, |s| {
        !s.pulls.list.detail_loading && !s.pulls.list.comments.review.is_empty()
    });
}

fn compose(rt: &mut Runtime<State>, text: &str) {
    rt.state.pulls.list.compose = TextEdit::new(text);
    step(rt);
}

#[test]
fn a_line_comment_targets_the_revision_side_and_line_shown_and_threads_are_visible() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    click(&mut rt, "prlk0");
    loaded(&mut rt, 1);
    comments_ready(&mut rt);
    assert_eq!(text_of(&rt, "prc-general0").as_deref(), Some("hello"));
    click(&mut rt, "prl-tab-files");
    step(&mut rt);
    assert_eq!(text_of(&rt, "prc-f0c10").as_deref(), Some("root"));
    assert_eq!(text_of(&rt, "prc-f0r11").as_deref(), Some("answer"));
    assert_eq!(text_of(&rt, "prc-f1c12").as_deref(), Some("old"));
    assert!(shows(&rt, "ligne 1") && shows(&rt, "ligne 3, version précédente"));
    click(&mut rt, "prl-f0l2");
    step(&mut rt);
    assert!(shows(&rt, "ligne 1 (nouveau côté) de f1.rs"));
    click(&mut rt, "prl-f0l1");
    step(&mut rt);
    assert!(shows(&rt, "ligne 1 (ancien côté) de f1.rs"));
    click(&mut rt, "prl-f0l2");
    compose(&mut rt, "please rename");
    click(&mut rt, "prc-send-line");
    settle(&mut rt, |s| {
        !s.pulls.list.sending
            && s.pulls
                .list
                .comments
                .review
                .iter()
                .any(|c| c.body == "please rename")
    });
    assert!(shows(&rt, "Commentaire publié."));
    assert!(shows(&rt, "please rename"));
    assert!(rect_of(&rt, "prc-box").is_none());
    assert_eq!(rt.state.pulls.list.compose.text, "");
}

#[test]
fn replies_join_their_thread_and_a_range_needs_two_lines_on_the_same_side() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    click(&mut rt, "prlk0");
    loaded(&mut rt, 1);
    comments_ready(&mut rt);
    click(&mut rt, "prl-tab-files");
    step(&mut rt);
    click(&mut rt, "prc-reply10");
    step(&mut rt);
    assert!(shows(&rt, "Réponse au fil"));
    assert!(rect_of(&rt, "prc-thread10").is_some() && rect_of(&rt, "prc-box").is_some());
    compose(&mut rt, "agreed");
    click(&mut rt, "prc-send-reply");
    settle(&mut rt, |s| {
        !s.pulls.list.sending
            && s.pulls
                .list
                .comments
                .review
                .iter()
                .any(|c| c.body == "agreed" && c.in_reply_to == Some(10))
    });
    assert!(shows(&rt, "agreed"));
    rt.state.pull_toggle_range();
    rt.state
        .pull_pick_line("f1.rs".to_owned(), 1, gitpoulpe_hosting::Side::Right);
    rt.state
        .pull_pick_line("f1.rs".to_owned(), 3, gitpoulpe_hosting::Side::Right);
    step(&mut rt);
    assert!(shows(&rt, "ligne 1–3 (nouveau côté) de f1.rs"));
    rt.state
        .pull_pick_line("f1.rs".to_owned(), 5, gitpoulpe_hosting::Side::Left);
    rt.state
        .pull_pick_line("f1.rs".to_owned(), 2, gitpoulpe_hosting::Side::Left);
    let pick = rt.state.pulls.list.pick.clone().unwrap();
    assert_eq!((pick.start, pick.line), (None, 2));
}

#[test]
fn a_stale_anchor_keeps_the_text_and_a_lost_answer_is_checked_before_any_retry() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    click(&mut rt, "prlk0");
    loaded(&mut rt, 1);
    rt.state
        .pulls
        .list
        .detail
        .as_mut()
        .unwrap()
        .summary
        .head_sha = "oldsha0".to_owned();
    click(&mut rt, "prl-tab-files");
    step(&mut rt);
    click(&mut rt, "prl-f0l2");
    compose(&mut rt, "stale thought");
    click(&mut rt, "prc-send-line");
    settle(&mut rt, |s| {
        !s.pulls.list.sending && s.pulls.list.send_error.is_some()
    });
    assert!(shows(&rt, "l'ancrage du commentaire est périmé"));
    assert_eq!(rt.state.pulls.list.compose.text, "stale thought");
    assert!(rt.state.pulls.list.unverified.is_none());
    click(&mut rt, "prc-cancel");
    click(&mut rt, "prl-tab-conversation");
    step(&mut rt);
    compose(&mut rt, "FLAKY retry");
    click(&mut rt, "prc-send-general");
    settle(&mut rt, |s| {
        !s.pulls.list.sending && s.pulls.list.unverified.is_some()
    });
    assert!(shows(&rt, "Le résultat de l'envoi est inconnu"));
    assert_eq!(rt.state.pulls.list.compose.text, "FLAKY retry");
    click(&mut rt, "prc-verify");
    settle(&mut rt, |s| {
        !s.pulls.list.verifying && s.pulls.list.unverified.is_none()
    });
    assert!(shows(&rt, "déjà publié : ne le renvoyez pas"));
    assert_eq!(rt.state.pulls.list.compose.text, "");
}

#[test]
fn diff_lines_are_numbered_from_each_hunk_header_on_the_side_they_belong_to() {
    use gitpoulpe_hosting::Side::{Left, Right};
    let lines = crate::pull_list::numbered_lines(
        "@@ -10,2 +40,3 @@\n ctx\n-gone\n+added\n+more\n\\ No newline at end of file\n@@ -90 +100 @@\n-a\n+b",
    );
    let anchors: Vec<_> = lines.iter().map(|l| l.anchor).collect();
    assert_eq!(
        anchors,
        [
            None,
            Some((40, Right)),
            Some((11, Left)),
            Some((41, Right)),
            Some((42, Right)),
            None,
            None,
            Some((90, Left)),
            Some((100, Right)),
        ]
    );
}

#[test]
fn a_comment_answer_arriving_after_another_pull_request_was_selected_is_dropped() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    rt.state.select_pull(1);
    loaded(&mut rt, 1);
    compose(&mut rt, "late note");
    rt.state
        .pull_send_comment(crate::pull_list::CommentKind::General);
    rt.state.select_pull(4);
    for _ in 0..60 {
        step(&mut rt);
        std::thread::sleep(Duration::from_millis(10));
    }
    let list = &rt.state.pulls.list;
    assert!(list.send_notice.is_none() && !list.sending);
    assert!(list.comments.issue.iter().all(|c| c.body != "late note") || list.selected == Some(4));
    assert!(list.send_error.is_none());
}

fn open_one(rt: &mut Runtime<State>, number: u64, row: usize) {
    click(rt, &format!("prlk{row}"));
    loaded(rt, number);
    settle(rt, |s| !s.pulls.list.comments.review.is_empty());
    click(rt, "prl-tab-files");
    step(rt);
}

fn add_to_draft(rt: &mut Runtime<State>, line_key: &str, text: &str) {
    click(rt, line_key);
    step(rt);
    compose(rt, text);
    click(rt, "prc-add-draft");
    step(rt);
}

fn draft_len(rt: &Runtime<State>) -> usize {
    rt.state.current_draft().map_or(0, |d| d.entries.len())
}

#[test]
fn a_multi_file_draft_is_submitted_as_one_review_with_the_chosen_decision_and_the_effective_account()
 {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    open_one(&mut rt, 1, 0);
    add_to_draft(&mut rt, "prl-f0l2", "first remark");
    add_to_draft(&mut rt, "prl-f1l2", "second remark");
    assert_eq!(draft_len(&rt), 2);
    assert!(shows(&rt, "En tant que Octo sur"));
    assert_eq!(
        text_of(&rt, "prv-count").as_deref(),
        Some("2 commentaires de ligne en attente")
    );
    rt.state.pulls.list.review_body = TextEdit::new("please fix");
    step(&mut rt);
    click(&mut rt, "prv-dec-changes");
    step(&mut rt);
    click(&mut rt, "prv-submit");
    settle(&mut rt, |s| {
        !s.pulls.list.review_sending && s.pulls.list.review_notice.is_some()
    });
    assert!(shows(
        &rt,
        "Revue publiée par Octo : changements demandés sur la révision sha1"
    ));
    assert_eq!(draft_len(&rt), 0);
    assert!(
        rt.state
            .pulls
            .list
            .reviews
            .iter()
            .any(|r| r.author == "Octo"
                && r.state == "CHANGES_REQUESTED"
                && r.body == "please fix")
    );
    assert_eq!(rt.state.pulls.list.review_body.text, "");
}

#[test]
fn drafts_stay_with_their_own_pull_request_when_switching_and_coming_back() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    open_one(&mut rt, 1, 0);
    add_to_draft(&mut rt, "prl-f0l2", "kept remark");
    rt.state.pulls.list.review_body = TextEdit::new("kept message");
    step(&mut rt);
    click(&mut rt, "prlk1");
    loaded(&mut rt, 4);
    assert_eq!(draft_len(&rt), 0);
    assert_eq!(rt.state.pulls.list.review_body.text, "");
    click(&mut rt, "prlk0");
    loaded(&mut rt, 1);
    assert_eq!(draft_len(&rt), 1);
    assert_eq!(rt.state.pulls.list.review_body.text, "kept message");
    assert_eq!(
        rt.state.current_draft().unwrap().entries[0].line.body,
        "kept remark"
    );
}

#[test]
fn an_update_during_the_review_flags_what_to_reread_without_losing_the_work_and_blocks_the_send() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    open_one(&mut rt, 1, 0);
    add_to_draft(&mut rt, "prl-f0l2", "remark one");
    add_to_draft(&mut rt, "prl-f1l2", "remark two");
    let key = rt.state.review_key().unwrap();
    {
        let draft = rt.state.pulls.list.drafts.get_mut(&key).unwrap();
        draft.head_sha = "older00".to_owned();
        draft.entries[0].patch_hash = 1;
    }
    rt.state.load_pull_detail();
    settle(&mut rt, |s| {
        !s.pulls.list.detail_loading && s.current_draft().is_some_and(|d| d.stale)
    });
    assert_eq!(draft_len(&rt), 2);
    let flags: Vec<bool> = rt
        .state
        .current_draft()
        .unwrap()
        .entries
        .iter()
        .map(|e| e.to_reread)
        .collect();
    assert_eq!(flags, [true, false]);
    assert!(shows(
        &rt,
        "a changé depuis le début du brouillon (révision older00)"
    ));
    assert_eq!(
        text_of(&rt, "prv-entry0").as_deref(),
        Some("remark one — à relire")
    );
    rt.state.pulls.list.review_body = TextEdit::new("msg");
    step(&mut rt);
    click(&mut rt, "prv-submit");
    step(&mut rt);
    assert!(shows(&rt, "adoptez la révision actuelle"));
    click(&mut rt, "prv-adopt");
    step(&mut rt);
    click(&mut rt, "prv-submit");
    step(&mut rt);
    assert!(shows(&rt, "1 commentaire(s) du brouillon sont à relire"));
    click(&mut rt, "prv-read0");
    step(&mut rt);
    click(&mut rt, "prv-submit");
    settle(&mut rt, |s| {
        !s.pulls.list.review_sending && s.pulls.list.review_notice.is_some()
    });
    assert!(shows(&rt, "Revue publiée par Octo"));
}

#[test]
fn a_missing_permission_an_obsolete_revision_and_a_lost_answer_never_look_like_a_success() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    open_one(&mut rt, 1, 0);
    rt.state.pulls.list.detail.as_mut().unwrap().summary.author = "Octo".to_owned();
    click(&mut rt, "prv-dec-approve");
    step(&mut rt);
    click(&mut rt, "prv-submit");
    step(&mut rt);
    assert!(review_error_has(
        &rt,
        "Vous êtes l'auteur de cette pull request"
    ));
    assert!(!rt.state.pulls.list.review_sending);
    rt.state.pulls.list.detail.as_mut().unwrap().summary.author = "author1".to_owned();
    rt.state
        .pulls
        .list
        .detail
        .as_mut()
        .unwrap()
        .summary
        .head_sha = "zzz9999".to_owned();
    rt.state.pulls.list.review_body = TextEdit::new("fine");
    step(&mut rt);
    click(&mut rt, "prv-submit");
    settle(&mut rt, |s| {
        !s.pulls.list.review_sending && s.pulls.list.review_error.is_some()
    });
    assert!(review_error_has(&rt, "a été mise à jour (zzz9999 → sha1)"));
    assert!(rt.state.pulls.list.review_notice.is_none());
    rt.state
        .pulls
        .list
        .detail
        .as_mut()
        .unwrap()
        .summary
        .head_sha = "sha1".to_owned();
    rt.state.pulls.list.drafts.clear();
    rt.state.pulls.list.review_body = TextEdit::new("FLAKY words");
    step(&mut rt);
    click(&mut rt, "prv-dec-comment");
    step(&mut rt);
    click(&mut rt, "prv-submit");
    settle(&mut rt, |s| {
        !s.pulls.list.review_sending && s.pulls.list.review_unverified
    });
    assert!(rt.state.pulls.list.review_unverified);
    click(&mut rt, "prv-verify");
    settle(&mut rt, |s| {
        !s.pulls.list.review_verifying && !s.pulls.list.review_unverified
    });
    assert!(review_notice_has(
        &rt,
        "La revue est déjà publiée : ne la renvoyez pas"
    ));
    click(&mut rt, "prv-discard");
    step(&mut rt);
    assert!(review_notice_has(&rt, "Brouillon de revue abandonné"));
    assert_eq!(rt.state.pulls.list.review_body.text, "");
}

#[test]
fn a_review_answer_arriving_after_another_pull_request_was_selected_is_dropped() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    rt.state.select_pull(1);
    loaded(&mut rt, 1);
    rt.state.pulls.list.review_body = TextEdit::new("late review");
    rt.state.pull_submit_review();
    assert!(rt.state.pulls.list.review_sending);
    rt.state.select_pull(4);
    for _ in 0..60 {
        step(&mut rt);
        std::thread::sleep(Duration::from_millis(10));
    }
    let list = &rt.state.pulls.list;
    assert!(list.review_notice.is_none() && !list.review_sending);
    assert!(list.reviews.iter().all(|r| r.body != "late review"));
}

fn checks_ready(rt: &mut Runtime<State>, number: u64) {
    loaded(rt, number);
    settle(rt, |s| !s.pulls.list.checks_loading);
}

#[test]
fn checks_are_shown_per_state_for_the_current_revision_and_absence_is_not_success() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    rt.state.select_pull(1);
    checks_ready(&mut rt, 1);
    assert!(shows(&rt, "révision sha1"));
    for count in [
        "1 échouée",
        "1 en attente",
        "1 annulée",
        "2 réussies",
        "1 ignorée",
    ] {
        assert!(shows(&rt, count), "{count}");
    }
    assert!(shows(&rt, "lint") && shows(&rt, "deploy") && shows(&rt, "e2e"));
    assert!(
        !shows(&rt, "legacy/ok (statut)"),
        "successes wait behind the toggle"
    );
    rt.take_url_requests();
    click(&mut rt, "chk-item1");
    step(&mut rt);
    assert_eq!(rt.take_url_requests(), ["https://ci.test/2"]);
    click(&mut rt, "chk-more");
    step(&mut rt);
    assert!(shows(&rt, "legacy/ok (statut)") && shows(&rt, "docs"));
    rt.state.select_pull(4);
    checks_ready(&mut rt, 4);
    assert!(shows(&rt, "Aucune vérification pour cette révision."));
    assert!(shows(&rt, "révision newsha"));
    assert!(rect_of(&rt, "chk-summary").is_none());
    rt.state.select_pull(2);
    checks_ready(&mut rt, 2);
    assert!(shows(&rt, "legacy/ci (statut)"));
}

#[test]
fn an_unavailable_api_and_a_new_revision_never_leave_old_results_looking_current() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    rt.state.select_pull(3);
    checks_ready(&mut rt, 3);
    assert!(shows(
        &rt,
        "Vérifications indisponibles : le serveur a répondu 500"
    ));
    assert!(rt.state.pulls.list.checks.is_none());
    assert!(!shows(&rt, "Aucune vérification"));
    rt.state.select_pull(1);
    checks_ready(&mut rt, 1);
    rt.state.pulls.list.checks.as_mut().unwrap().sha = "oldsha0".to_owned();
    rt.state.load_pull_detail();
    settle(&mut rt, |s| {
        !s.pulls.list.detail_loading
            && !s.pulls.list.checks_loading
            && s.pulls.list.checks_note.is_some()
    });
    assert!(shows(
        &rt,
        "Les vérifications de la révision précédente (oldsha0) ne s'appliquent plus à la révision actuelle (sha1)."
    ));
    assert_eq!(rt.state.pulls.list.checks.as_ref().unwrap().sha, "sha1");
    rt.state.select_pull(2);
    checks_ready(&mut rt, 2);
    assert!(rt.state.pulls.list.checks_note.is_none());
    let late = gitpoulpe_hosting::Checks {
        sha: "sha1".to_owned(),
        items: Vec::new(),
        truncated: false,
    };
    let request = rt.state.pulls.list.checks_request;
    assert!(
        !rt.state
            .apply_pull_checks(request, 2, "sha1".to_owned(), Ok(late.clone()))
    );
    assert!(
        !rt.state
            .apply_pull_checks(request - 1, 2, "sha2".to_owned(), Ok(late.clone()))
    );
    assert!(
        !rt.state
            .apply_pull_checks(request, 1, "sha2".to_owned(), Ok(late))
    );
    assert_eq!(rt.state.pulls.list.checks.as_ref().unwrap().sha, "sha2");
}

#[test]
fn old_check_results_are_dropped_at_once_on_a_new_revision_and_on_a_failed_refresh() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    rt.state.select_pull(1);
    checks_ready(&mut rt, 1);
    assert!(rt.state.pulls.list.checks.is_some());
    rt.state
        .pulls
        .list
        .detail
        .as_mut()
        .unwrap()
        .summary
        .head_sha = "sha2".to_owned();
    rt.state.load_pull_checks();
    assert!(rt.state.pulls.list.checks.is_none());
    assert!(rt.state.pulls.list.checks_note.is_some());
    for _ in 0..20 {
        step(&mut rt);
        std::thread::sleep(Duration::from_millis(10));
    }
    rt.state
        .pulls
        .list
        .detail
        .as_mut()
        .unwrap()
        .summary
        .head_sha = "sha5".to_owned();
    rt.state.pulls.list.checks = Some(gitpoulpe_hosting::Checks {
        sha: "sha5".to_owned(),
        items: vec![gitpoulpe_hosting::Check {
            name: "stale-green".to_owned(),
            state: gitpoulpe_hosting::CheckState::Success,
            url: String::new(),
            legacy: false,
        }],
        truncated: false,
    });
    rt.state.load_pull_checks();
    settle(&mut rt, |s| !s.pulls.list.checks_loading);
    assert!(rt.state.pulls.list.checks.is_none());
    assert!(!shows(&rt, "stale-green"));
    assert!(shows(&rt, "Vérifications indisponibles"));
}

fn merge_ready(rt: &mut Runtime<State>, number: u64) {
    loaded(rt, number);
    settle(rt, |s| {
        !s.pulls.list.merge_loading && s.pulls.list.merge.is_some()
    });
}

#[test]
fn only_allowed_methods_are_offered_with_their_effect_and_the_blockers_are_listed() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    rt.state.select_pull(1);
    merge_ready(&mut rt, 1);
    assert!(rect_of(&rt, "mrg-merge").is_some() && rect_of(&rt, "mrg-squash").is_some());
    assert!(rect_of(&rt, "mrg-rebase").is_none());
    assert!(shows(
        &rt,
        "Tous les commits de la branche restent dans l'historique."
    ));
    assert!(shows(
        &rt,
        "Les commits de la branche sont regroupés en un commit sur la cible."
    ));
    assert!(shows(&rt, "« lint » n'a pas réussi"));
    assert!(shows(&rt, "« missing » est absente"));
    assert!(shows(&rt, "1 approbation(s) sur 2 requise(s)"));
    rt.state.select_pull(4);
    merge_ready(&mut rt, 4);
    assert!(rect_of(&rt, "mrg-merge").is_none());
    assert!(shows(&rt, "La pull request est un brouillon."));
}

#[test]
fn a_merge_needs_a_confirmation_names_the_revision_and_is_verified_on_the_remote() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    rt.state.select_pull(1);
    merge_ready(&mut rt, 1);
    click(&mut rt, "mrg-squash");
    step(&mut rt);
    assert!(shows(
        &rt,
        "Un seul commit sur la révision sha1 : confirmez pour fusionner."
    ));
    assert!(rt.state.pulls.list.merge_notice.is_none() && !rt.state.pulls.list.merging);
    click(&mut rt, "mrg-cancel");
    step(&mut rt);
    assert!(rt.state.pulls.list.merge_confirm.is_none());
    click(&mut rt, "mrg-merge");
    step(&mut rt);
    click(&mut rt, "mrg-confirm");
    settle(&mut rt, |s| {
        !s.pulls.list.merging && s.pulls.list.merge_notice.is_some()
    });
    assert!(shows(&rt, "Pull request fusionnée (commit mergesh)."));
    settle(&mut rt, |s| {
        !s.pulls.list.detail_loading
            && s.pulls
                .list
                .detail
                .as_ref()
                .is_some_and(|d| d.summary.state == gitpoulpe_hosting::PullState::Merged)
    });
    assert!(shows(&rt, "Fusionnée · par author1"));
}

#[test]
fn an_advanced_revision_a_refusal_and_a_lost_answer_never_pass_for_a_completed_merge() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    rt.state.select_pull(1);
    merge_ready(&mut rt, 1);
    rt.state.pulls.list.merge.as_mut().unwrap().head_sha = "oldsha0".to_owned();
    click(&mut rt, "mrg-merge");
    step(&mut rt);
    click(&mut rt, "mrg-confirm");
    settle(&mut rt, |s| {
        !s.pulls.list.merging && s.pulls.list.merge_error.is_some()
    });
    assert!(shows(&rt, "a été mise à jour (oldsha0 → sha1)"));
    assert!(rt.state.pulls.list.merge_notice.is_none());
    rt.state.pulls.list.merge.as_mut().unwrap().head_sha = "sha1".to_owned();
    click(&mut rt, "mrg-merge");
    step(&mut rt);
    rt.state.pulls.list.merge_message = TextEdit::new("FLAKY merge");
    step(&mut rt);
    click(&mut rt, "mrg-confirm");
    settle(&mut rt, |s| {
        !s.pulls.list.merging && s.pulls.list.merge_unverified
    });
    assert!(shows(&rt, "Le résultat de la fusion est inconnu"));
    assert!(rt.state.pulls.list.merge_notice.is_none());
    click(&mut rt, "mrg-verify");
    settle(&mut rt, |s| !s.pulls.list.merge_unverified);
    assert!(shows(
        &rt,
        "La pull request est fusionnée : ne relancez pas la fusion."
    ));
}

#[test]
fn a_server_refusal_is_shown_as_a_refusal_and_a_late_merge_answer_is_dropped() {
    let fake = Fake::start();
    let (_dir, mut rt) = list_screen(&fake);
    rt.state.select_pull(1);
    merge_ready(&mut rt, 1);
    rt.state.pulls.list.merge.as_mut().unwrap().methods =
        vec![gitpoulpe_hosting::MergeMethod::Merge];
    rt.state.pulls.list.merge_confirm = Some(gitpoulpe_hosting::MergeMethod::Merge);
    rt.state.pull_merge_confirmed();
    assert!(rt.state.pulls.list.merging);
    rt.state.select_pull(4);
    for _ in 0..60 {
        step(&mut rt);
        std::thread::sleep(Duration::from_millis(10));
    }
    let list = &rt.state.pulls.list;
    assert!(!list.merging && list.merge_notice.is_none());
    rt.state.select_pull(2);
    merge_ready(&mut rt, 2);
    assert!(rect_of(&rt, "mrg-merge").is_none());
    assert!(shows(&rt, "La pull request n'est plus ouverte"));
}

fn review_error_has(rt: &Runtime<State>, needle: &str) -> bool {
    rt.state
        .pulls
        .list
        .review_error
        .as_deref()
        .is_some_and(|e| e.contains(needle))
}

fn review_notice_has(rt: &Runtime<State>, needle: &str) -> bool {
    rt.state
        .pulls
        .list
        .review_notice
        .as_deref()
        .is_some_and(|e| e.contains(needle))
}

fn git_out(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

struct BigPull {
    _source: tempfile::TempDir,
    local: tempfile::TempDir,
    base: String,
    head: String,
}

fn big_pull(fake: &Fake, files: usize) -> BigPull {
    let source = tempfile::tempdir().unwrap();
    let s = source.path();
    git(s, &["init", "-q", "-b", "main"]);
    std::fs::write(s.join("readme.txt"), "base\n").unwrap();
    git(s, &["add", "-A"]);
    git(s, &["commit", "-q", "-m", "root"]);
    let local = repo(&format!("{}/acme/proj.git", fake.base()), "main");
    git(
        local.path(),
        &[
            "config",
            &format!("url.file://{}.insteadOf", s.display()),
            &format!("{}/acme/proj.git", fake.base()),
        ],
    );
    git(s, &["checkout", "-q", "-b", "pr"]);
    for i in 0..files {
        let dir = s.join(format!("d{}", i % 30));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("f{i}.txt")), format!("{i}\n")).unwrap();
    }
    git(s, &["add", "-A"]);
    git(s, &["commit", "-q", "-m", "Huge change"]);
    let head = git_out(s, &["rev-parse", "HEAD"]);
    git(s, &["update-ref", "refs/pull/1/head", &head]);
    git(s, &["checkout", "-q", "main"]);
    std::fs::write(s.join("later.txt"), "later\n").unwrap();
    git(s, &["add", "-A"]);
    git(s, &["commit", "-q", "-m", "main moved"]);
    let base = git_out(s, &["rev-parse", "HEAD"]);
    BigPull {
        _source: source,
        local,
        base,
        head,
    }
}

#[test]
fn a_pull_request_beyond_the_api_cap_is_read_entirely_from_git_revisions() {
    let fake = Fake::start();
    let big = big_pull(&fake, 3100);
    let mut rt = screen(big.local.path(), &fake, &["Octo"]);
    click(&mut rt, "pr-list");
    settle(&mut rt, |s| s.pulls.list.open && !s.pulls.list.loading);
    rt.state.select_pull(1);
    loaded(&mut rt, 1);
    {
        let summary = &mut rt.state.pulls.list.detail.as_mut().unwrap().summary;
        summary.head_sha = big.head.clone();
        summary.base_sha = big.base.clone();
        summary.base_ref = "main".to_owned();
    }
    step(&mut rt);
    click(&mut rt, "prl-git");
    settle(&mut rt, |s| s.inspected.is_some());
    let inspected = rt.state.inspected.as_ref().unwrap();
    assert_eq!(inspected.detail.commit.oid, big.head);
    assert_eq!(inspected.entries.len(), 3100);
    assert!(!rt.state.pulls.open && !rt.state.pulls.list.open);
}

#[test]
fn an_inaccessible_revision_reports_a_limit_and_loads_nothing() {
    let fake = Fake::start();
    let big = big_pull(&fake, 3);
    let mut rt = screen(big.local.path(), &fake, &["Octo"]);
    click(&mut rt, "pr-list");
    settle(&mut rt, |s| s.pulls.list.open && !s.pulls.list.loading);
    rt.state.select_pull(1);
    loaded(&mut rt, 1);
    {
        let summary = &mut rt.state.pulls.list.detail.as_mut().unwrap().summary;
        summary.head_sha = "d".repeat(40);
        summary.base_sha = big.base.clone();
        summary.base_ref = "main".to_owned();
    }
    step(&mut rt);
    click(&mut rt, "prl-git");
    settle(&mut rt, |s| s.notice.is_some());
    assert!(rt.state.inspected.is_none());
    assert!(rt.state.notice.as_deref().is_some_and(|n| {
        n.contains("Impossible de lire la pull request depuis Git")
            && n.contains("n'a pas été chargée en entier")
    }));
}
