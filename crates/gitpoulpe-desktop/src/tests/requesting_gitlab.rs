use gitpoulpe_core::{Connection, Provider};
use rune::TextEdit;

use super::requesting::{Fake, click, loaded, repo, screen_as, settle, shows};
use super::{rect_of, step};

fn gitlab(fake: &Fake, login: &str) -> Connection {
    Connection {
        id: format!("127.0.0.1:{}/{}", fake.port, login.to_lowercase()),
        provider: Provider::Gitlab,
        api: format!("{}/api/v4", fake.base()),
        web: fake.base(),
        login: login.to_owned(),
        scopes: Vec::new(),
    }
}

const PROJECT: &str = "acme/platform/api";

fn open(fake: &Fake, branch: &str) -> (tempfile::TempDir, rune::Runtime<crate::state::State>) {
    let dir = repo(&format!("{}/{PROJECT}.git", fake.base()), branch);
    let rt = screen_as(dir.path(), fake, &["Octo"], gitlab);
    (dir, rt)
}

#[test]
fn a_merge_request_is_created_as_a_draft_once_for_a_project_in_nested_groups() {
    let fake = Fake::start_script("fake_gitlab.py");
    let (_dir, mut rt) = open(&fake, "feature/x");
    assert!(shows(&rt, "Merge request · feature/x"));
    assert!(shows(
        &rt,
        "acme/platform/api:feature/x → acme/platform/api:main"
    ));
    assert!(shows(&rt, "2 commit(s) d'avance, 1 de retard"));
    assert!(shows(&rt, "modified src/a.rs (+2 −1)"));
    assert!(shows(&rt, "Créer la merge request"));
    rt.state.pulls.title = TextEdit::new("Add the thing");
    step(&mut rt);
    click(&mut rt, "pr-draft");
    step(&mut rt);
    click(&mut rt, "pr-create");
    settle(&mut rt, |s| !s.pulls.busy && s.pulls.created.is_some());
    assert!(shows(&rt, "Merge request #10 créée (brouillon)"));
    assert!(rect_of(&rt, "pr-create").is_none());
    assert!(shows(&rt, "Une merge request est déjà ouverte : #10"));
    rt.state.create_pull();
    assert!(!rt.state.pulls.busy);
}

#[test]
fn an_existing_merge_request_and_an_unpublished_branch_offer_no_create_button() {
    let fake = Fake::start_script("fake_gitlab.py");
    let (_dir, rt) = open(&fake, "feature/dup");
    assert!(shows(&rt, "Une merge request est déjà ouverte : #5"));
    assert!(rect_of(&rt, "pr-create").is_none());
    let (_other, rt) = open(&fake, "local-only");
    assert!(shows(&rt, "La branche local-only n'est pas publiée"));
    assert!(rect_of(&rt, "pr-create").is_none());
}

#[test]
fn merge_requests_are_listed_and_one_shows_its_revisions_files_and_the_missing_features() {
    let fake = Fake::start_script("fake_gitlab.py");
    let (_dir, mut rt) = open(&fake, "feature/x");
    click(&mut rt, "pr-list");
    settle(&mut rt, |s| s.pulls.list.open && !s.pulls.list.loading);
    assert!(shows(&rt, "Merge requests · 5 résultat(s)"));
    assert!(shows(&rt, "#9 MR 9") && shows(&rt, "#5 Work in progress"));
    click(&mut rt, "prlk2");
    loaded(&mut rt, 7);
    assert!(shows(&rt, "feature/y (ddddddd) → main (bbbbbbb)"));
    assert!(shows(&rt, "4 fichier(s)") && shows(&rt, "2 commit(s)"));
    assert!(shows(&rt, "+more") && shows(&rt, "-old"));
    assert!(shows(&rt, "Second — Octo"));
    assert!(shows(
        &rt,
        "pas encore disponibles pour les merge requests GitLab"
    ));
    assert!(!shows(&rt, "commentaire(s) généraux"));
    let list = &rt.state.pulls.list;
    assert!(list.checks.is_none() && list.checks_error.is_none() && !list.checks_loading);
    assert!(list.merge.is_none() && list.merge_error.is_none() && !list.merge_loading);
    assert!(list.reviews.is_empty() && list.review_error.is_none());
}

#[test]
fn a_partial_file_list_from_an_older_server_is_flagged_next_to_the_git_reader() {
    let fake = Fake::start_script("fake_gitlab.py");
    let (_dir, mut rt) = open(&fake, "feature/x");
    click(&mut rt, "pr-list");
    settle(&mut rt, |s| s.pulls.list.open && !s.pulls.list.loading);
    click(&mut rt, "prlk1");
    loaded(&mut rt, 8);
    assert!(shows(&rt, "GitLab n'a pas renvoyé la liste complète"));
    assert!(shows(&rt, "(liste tronquée)"));
    assert!(rect_of(&rt, "prl-git").is_some());
}
