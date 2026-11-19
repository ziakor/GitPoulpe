mod common;

use std::path::Path;

use common::{commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{Cancel, Git, NetError, PullRangeError, ReviewRefs};

struct Env {
    source: tempfile::TempDir,
    local: tempfile::TempDir,
    git: Git,
    info: SessionInfo,
    base: String,
    head: String,
    root: String,
}

fn out(dir: &Path, args: &[&str]) -> String {
    String::from_utf8_lossy(&run_git(dir, args))
        .trim()
        .to_owned()
}

async fn env(files: usize) -> Env {
    let source = init_repo();
    write(source.path(), "readme.txt", "base\n");
    commit_all(source.path(), "root");
    let root = out(source.path(), &["rev-parse", "HEAD"]);
    let local = init_repo();
    run_git(
        local.path(),
        &[
            "remote",
            "add",
            "origin",
            &format!("file://{}", source.path().display()),
        ],
    );
    run_git(local.path(), &["fetch", "-q", "origin", "main"]);
    run_git(
        local.path(),
        &["checkout", "-q", "-B", "main", "FETCH_HEAD"],
    );
    run_git(source.path(), &["checkout", "-q", "-b", "pr"]);
    for i in 0..files {
        write(
            source.path(),
            &format!("dir{}/f{i}.txt", i % 40),
            &format!("{i}\n"),
        );
    }
    commit_all(source.path(), "big change");
    let head = out(source.path(), &["rev-parse", "HEAD"]);
    run_git(source.path(), &["update-ref", "refs/pull/7/head", &head]);
    run_git(source.path(), &["checkout", "-q", "main"]);
    write(source.path(), "later.txt", "later\n");
    commit_all(source.path(), "main moved on");
    let base = out(source.path(), &["rev-parse", "HEAD"]);
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null");
    let info = git.open(local.path(), &Cancel::new()).await.unwrap();
    Env {
        source,
        local,
        git,
        info,
        base,
        head,
        root,
    }
}

async fn range(
    e: &Env,
    number: u64,
    base_ref: &str,
    base: &str,
    head: &str,
) -> Result<gitpoulpe_git::PullRange, PullRangeError> {
    e.git
        .prepare_pull_range(
            &e.info,
            "origin",
            ReviewRefs::Github,
            number,
            base_ref,
            base,
            head,
            &Cancel::new(),
            |_| {},
        )
        .await
}

#[tokio::test]
async fn a_pull_request_of_3200_files_is_read_completely_from_the_exact_revisions() {
    let e = env(3200).await;
    let before = gitpoulpe_git_exists(&e);
    assert!(!before);
    let found = range(&e, 7, "main", &e.base, &e.head).await.unwrap();
    assert!(found.fetched);
    assert_eq!(found.head, e.head);
    assert_eq!(found.merge_base, e.root);
    let entries = e
        .git
        .commit_files(&e.info, &found.head, &found.merge_base)
        .await
        .unwrap();
    assert_eq!(entries.len(), 3200);
    assert!(
        entries
            .iter()
            .all(|c| c.path.to_string_lossy().ends_with(".txt"))
    );
    let again = range(&e, 7, "main", &e.base, &e.head).await.unwrap();
    assert!(!again.fetched);
    assert_eq!(again.merge_base, e.root);
    let _ = &e.source;
}

#[tokio::test]
async fn a_missing_revision_a_missing_pull_ref_and_an_unrelated_base_report_a_limit_and_never_a_partial_result()
 {
    let e = env(5).await;
    let ghost = "d".repeat(40);
    let lost = range(&e, 7, "main", &e.base, &ghost).await.unwrap_err();
    assert!(matches!(lost, PullRangeError::Inaccessible(_)), "{lost:?}");
    assert!(lost.to_string().contains("n'a pas été chargée en entier"));
    let no_ref = range(&e, 99, "main", &e.base, &"c".repeat(40))
        .await
        .unwrap_err();
    assert!(
        matches!(&no_ref, PullRangeError::Net(NetError::Refused(m)) if m.contains("couldn't find remote ref")),
        "{no_ref:?}"
    );
    run_git(e.source.path(), &["checkout", "-q", "--orphan", "other"]);
    write(e.source.path(), "o.txt", "o\n");
    commit_all(e.source.path(), "orphan");
    let orphan = out(e.source.path(), &["rev-parse", "HEAD"]);
    assert_eq!(
        range(&e, 7, "other", &orphan, &e.head).await.unwrap_err(),
        PullRangeError::NoCommonAncestor
    );
}

#[tokio::test]
async fn unsafe_inputs_are_refused_before_git_runs() {
    let e = env(2).await;
    for (number, base_ref, base, head) in [
        (0, "main", e.base.as_str(), e.head.as_str()),
        (7, "../x", e.base.as_str(), e.head.as_str()),
        (7, "-evil", e.base.as_str(), e.head.as_str()),
        (7, "main", "zz", e.head.as_str()),
        (7, "main", e.base.as_str(), "not-a-sha"),
    ] {
        assert_eq!(
            range(&e, number, base_ref, base, head).await.unwrap_err(),
            PullRangeError::InvalidInput,
            "{number} {base_ref} {base} {head}"
        );
    }
    let unknown = e
        .git
        .prepare_pull_range(
            &e.info,
            "nope",
            ReviewRefs::Github,
            7,
            "main",
            &e.base,
            &e.head,
            &Cancel::new(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert!(
        matches!(unknown, PullRangeError::Net(NetError::Refused(_))),
        "{unknown:?}"
    );
}

fn gitpoulpe_git_exists(e: &Env) -> bool {
    let output = common::git_output(
        e.local.path(),
        &["cat-file", "-e", &format!("{}^{{commit}}", e.head)],
    );
    output.status.success()
}

#[tokio::test]
async fn a_gitlab_merge_request_is_fetched_from_its_own_ref_namespace() {
    let e = env(5).await;
    run_git(
        e.source.path(),
        &["update-ref", "refs/merge-requests/8/head", &e.head],
    );
    let cancel = Cancel::new();
    let fetch = |refs| {
        e.git.prepare_pull_range(
            &e.info,
            "origin",
            refs,
            8,
            "main",
            &e.base,
            &e.head,
            &cancel,
            |_| {},
        )
    };
    assert!(matches!(
        fetch(ReviewRefs::Github).await.unwrap_err(),
        PullRangeError::Net(NetError::Refused(m)) if m.contains("couldn't find remote ref")
    ));
    let found = fetch(ReviewRefs::Gitlab).await.unwrap();
    assert!(found.fetched);
    assert_eq!(
        (found.head, found.merge_base),
        (e.head.clone(), e.root.clone())
    );
}
