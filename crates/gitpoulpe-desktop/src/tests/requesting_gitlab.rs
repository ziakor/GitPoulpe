use gitpoulpe_core::{Connection, Provider};
use rune::TextEdit;

use super::requesting::{Fake, click, loaded, repo, screen_as, settle, shows, text_of};
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
fn merge_requests_are_listed_and_one_shows_its_revisions_files_and_commits() {
    let fake = Fake::start_script("fake_gitlab.py");
    let (_dir, mut rt) = open(&fake, "feature/x");
    click(&mut rt, "pr-list");
    settle(&mut rt, |s| s.pulls.list.open && !s.pulls.list.loading);
    assert!(shows(&rt, "Merge requests"));
    assert_eq!(text_of(&rt, "prl-count").as_deref(), Some("5"));
    assert!(shows(&rt, "#9 · ouverte · octo") && shows(&rt, "Work in progress"));
    click(&mut rt, "prlk2");
    loaded(&mut rt, 7);
    assert!(shows(&rt, "feature/y (ddddddd) → main (bbbbbbb)"));
    assert!(shows(&rt, "Fichiers 4") && shows(&rt, "Commits 2"));
    click(&mut rt, "prl-tab-files");
    step(&mut rt);
    assert!(shows(&rt, "more") && shows(&rt, "old"));
    click(&mut rt, "prl-tab-commits");
    step(&mut rt);
    assert!(shows(&rt, "Second") && shows(&rt, "Octo"));
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
    click(&mut rt, "prl-tab-files");
    step(&mut rt);
    assert!(shows(&rt, "(liste tronquée)"));
    assert!(rect_of(&rt, "prl-git").is_some());
}

fn review_screen(
    fake: &Fake,
    number: u64,
) -> (tempfile::TempDir, rune::Runtime<crate::state::State>) {
    let (dir, mut rt) = open(fake, "feature/x");
    click(&mut rt, "pr-list");
    settle(&mut rt, |s| s.pulls.list.open && !s.pulls.list.loading);
    rt.state.select_pull(number);
    loaded(&mut rt, number);
    settle(&mut rt, |s| {
        let l = &s.pulls.list;
        l.merge.is_some() && l.checks.is_some() && !l.comments.review.is_empty()
    });
    (dir, rt)
}

#[test]
fn comments_checks_and_merge_rules_of_a_merge_request_are_read_from_gitlab() {
    let fake = Fake::start_script("fake_gitlab.py");
    let (_dir, mut rt) = review_screen(&fake, 20);
    assert_eq!(
        text_of(&rt, "prc-general0").as_deref(),
        Some("General remark")
    );
    click(&mut rt, "prl-tab-files");
    step(&mut rt);
    let general = &rt.state.pulls.list.comments.issue;
    assert_eq!(general.len(), 1);
    assert_eq!(general[0].body, "General remark");
    assert!(shows(&rt, "Fix this"));
    let list = &rt.state.pulls.list;
    assert_eq!(list.checks.as_ref().unwrap().items.len(), 6);
    let merge = list.merge.as_ref().unwrap();
    assert!(merge.blockers.is_empty() && merge.can_attempt());
    assert_eq!(merge.methods.len(), 2);
    assert!(list.checks_error.is_none() && list.merge_error.is_none());
    assert!(rect_of(&rt, "mrg-merge").is_some() && rect_of(&rt, "mrg-squash").is_some());
    assert!(rect_of(&rt, "prv-dec-changes").is_none());
    assert!(shows(
        &rt,
        "Demander des changements n'est pas possible par l'API GitLab"
    ));
}

#[test]
fn gitlab_blockers_are_shown_next_to_the_merge_buttons() {
    let fake = Fake::start_script("fake_gitlab.py");
    let (_dir, mut rt) = review_screen(&fake, 20);
    rt.state.select_pull(21);
    loaded(&mut rt, 21);
    settle(&mut rt, |s| {
        s.pulls
            .list
            .merge
            .as_ref()
            .is_some_and(|m| m.head_sha.starts_with("33"))
    });
    assert!(shows(&rt, "1 approbation(s) sur 2 requise(s)"));
    rt.state.select_pull(22);
    loaded(&mut rt, 22);
    settle(&mut rt, |s| {
        s.pulls
            .list
            .merge
            .as_ref()
            .is_some_and(|m| m.head_sha.starts_with("44"))
    });
    assert!(shows(&rt, "Des conflits de fusion doivent être résolus"));
}

#[test]
fn a_merge_request_is_merged_in_two_steps_and_confirmed_by_the_remote() {
    let fake = Fake::start_script("fake_gitlab.py");
    let (_dir, mut rt) = review_screen(&fake, 20);
    click(&mut rt, "mrg-merge");
    step(&mut rt);
    assert!(shows(
        &rt,
        "Commit de fusion sur la révision 2222222 : confirmez pour fusionner."
    ));
    assert!(rt.state.pulls.list.merge_notice.is_none() && !rt.state.pulls.list.merging);
    click(&mut rt, "mrg-confirm");
    settle(&mut rt, |s| {
        !s.pulls.list.merging && s.pulls.list.merge_notice.is_some()
    });
    assert!(shows(&rt, "Merge request fusionnée (commit mmmmmmm)."));
}

#[test]
fn a_general_comment_is_sent_to_the_merge_request() {
    let fake = Fake::start_script("fake_gitlab.py");
    let (_dir, mut rt) = review_screen(&fake, 20);
    rt.state.pulls.list.compose = TextEdit::new("Thanks for this");
    step(&mut rt);
    click(&mut rt, "prc-send-general");
    settle(&mut rt, |s| {
        !s.pulls.list.sending && s.pulls.list.compose.text.is_empty()
    });
    let sent = rt.state.pulls.list.comments.issue.last().unwrap();
    assert_eq!(
        (sent.author.as_str(), sent.body.as_str()),
        ("Octo", "Thanks for this")
    );
    assert!(rt.state.pulls.list.send_error.is_none());
}
