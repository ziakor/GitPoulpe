mod common;

use common::{commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::{SessionInfo, mask_url, url_has_secret};
use gitpoulpe_git::{Cancel, Git, RemoteError, validate_remote_url};

struct Fixture {
    repo: tempfile::TempDir,
    git: Git,
    info: SessionInfo,
}

async fn fixture() -> Fixture {
    let repo = init_repo();
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null");
    write(repo.path(), "a.txt", "base\n");
    commit_all(repo.path(), "base");
    let info = git.open(repo.path(), &Cancel::new()).await.unwrap();
    Fixture { repo, git, info }
}

impl Fixture {
    fn out(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&run_git(self.repo.path(), args))
            .trim()
            .to_owned()
    }
    fn config(&self) -> String {
        std::fs::read_to_string(self.repo.path().join(".git/config")).unwrap()
    }
}

#[tokio::test]
async fn remotes_and_their_read_and_write_urls_match_git_remote_v() {
    let f = fixture().await;
    f.out(&["remote", "add", "origin", "https://example.org/a.git"]);
    f.out(&["remote", "add", "mirror", "git@example.org:b.git"]);
    f.out(&[
        "remote",
        "set-url",
        "--push",
        "mirror",
        "ssh://example.org/push.git",
    ]);
    let remotes = f.git.list_remotes(&f.info).await.unwrap();
    let mut seen = Vec::new();
    for r in &remotes {
        for u in &r.fetch_urls {
            seen.push(format!("{}\t{} (fetch)", r.name, u));
        }
        for u in r.push_targets() {
            seen.push(format!("{}\t{} (push)", r.name, u));
        }
    }
    seen.sort();
    let mut expected: Vec<String> = f
        .out(&["remote", "-v"])
        .lines()
        .map(str::to_owned)
        .collect();
    expected.sort();
    assert_eq!(seen, expected);
    let mirror = remotes.iter().find(|r| r.name == "mirror").unwrap();
    assert_eq!(mirror.push_urls, ["ssh://example.org/push.git"]);
}

#[tokio::test]
async fn a_repository_without_remote_lists_none() {
    let f = fixture().await;
    assert!(f.git.list_remotes(&f.info).await.unwrap().is_empty());
}

#[tokio::test]
async fn adding_a_colliding_or_invalid_remote_never_touches_the_existing_configuration() {
    let f = fixture().await;
    f.git
        .add_remote(&f.info, "origin", "https://example.org/a.git")
        .await
        .unwrap();
    let before = f.config();
    assert_eq!(
        f.git
            .add_remote(&f.info, "origin", "https://evil.example/x.git")
            .await
            .unwrap_err(),
        RemoteError::AlreadyExists
    );
    for name in [
        "", "-x", "a b", "a..b", "@", "x\ny", "a/b", "end.lock", "we~ird",
    ] {
        assert_eq!(
            f.git
                .add_remote(&f.info, name, "https://example.org/z.git")
                .await
                .unwrap_err(),
            RemoteError::InvalidName,
            "{name:?}"
        );
    }
    for url in [
        "",
        "-oProxyCommand=touch /tmp/x",
        "ext::sh -c touch% /tmp/x",
        "fd::17/repo",
        "ssh://-oProxyCommand=x/repo",
        "ssh://user@-oProxyCommand=x/repo",
        "https://",
        "ftp://example.org/x",
        "https://exa mple.org/x",
        "https://example.org/x\nsecond",
        "nonsense",
    ] {
        assert!(
            matches!(
                f.git.add_remote(&f.info, "extra", url).await,
                Err(RemoteError::InvalidUrl(_))
            ),
            "{url:?}"
        );
    }
    assert_eq!(f.config(), before);
}

#[tokio::test]
async fn the_usual_url_forms_are_accepted() {
    for url in [
        "https://example.org/a.git",
        "http://example.org/a.git",
        "ssh://git@example.org:2222/a.git",
        "git://example.org/a.git",
        "file:///srv/repo.git",
        "git@example.org:team/a.git",
        "example.org:a.git",
        "/srv/repo.git",
        "../sibling",
        "./sibling",
        "https://user:token@example.org/a.git",
    ] {
        assert_eq!(validate_remote_url(url), Ok(()), "{url:?}");
    }
}

#[tokio::test]
async fn changing_a_url_is_explicit_checked_against_the_display_and_never_loses_extra_urls() {
    let f = fixture().await;
    let added = f
        .git
        .add_remote(&f.info, "origin", "https://example.org/a.git")
        .await
        .unwrap();
    let updated = f
        .git
        .set_remote_url(
            &f.info,
            "origin",
            "https://example.org/b.git",
            &added.fetch_urls,
        )
        .await
        .unwrap();
    assert_eq!(updated.fetch_urls, ["https://example.org/b.git"]);
    assert_eq!(
        f.git
            .set_remote_url(
                &f.info,
                "origin",
                "https://example.org/c.git",
                &added.fetch_urls
            )
            .await
            .unwrap_err(),
        RemoteError::Changed
    );
    assert!(matches!(
        f.git
            .set_remote_url(&f.info, "origin", "nonsense", &updated.fetch_urls)
            .await,
        Err(RemoteError::InvalidUrl(_))
    ));
    assert_eq!(
        f.out(&["config", "--get", "remote.origin.url"]),
        "https://example.org/b.git"
    );
    f.out(&[
        "config",
        "--add",
        "remote.origin.url",
        "https://example.org/second.git",
    ]);
    let both = f.git.list_remotes(&f.info).await.unwrap()[0]
        .fetch_urls
        .clone();
    assert_eq!(
        f.git
            .set_remote_url(&f.info, "origin", "https://example.org/d.git", &both)
            .await
            .unwrap_err(),
        RemoteError::MultipleUrls
    );
    assert_eq!(
        f.out(&["config", "--get-all", "remote.origin.url"])
            .lines()
            .count(),
        2
    );
    assert_eq!(
        f.git
            .set_remote_url(&f.info, "ghost", "https://example.org/x.git", &[])
            .await
            .unwrap_err(),
        RemoteError::NoSuchRemote
    );
}

#[tokio::test]
async fn a_separate_push_url_can_be_set_and_cleared() {
    let f = fixture().await;
    f.git
        .add_remote(&f.info, "origin", "https://example.org/a.git")
        .await
        .unwrap();
    let set = f
        .git
        .set_remote_push_url(&f.info, "origin", Some("ssh://example.org/p.git"), &[])
        .await
        .unwrap();
    assert_eq!(set.push_urls, ["ssh://example.org/p.git"]);
    assert_eq!(
        f.out(&["remote", "get-url", "--push", "origin"]),
        "ssh://example.org/p.git"
    );
    assert_eq!(
        f.git
            .set_remote_push_url(&f.info, "origin", None, &[])
            .await
            .unwrap_err(),
        RemoteError::Changed
    );
    let cleared = f
        .git
        .set_remote_push_url(&f.info, "origin", None, &set.push_urls)
        .await
        .unwrap();
    assert!(cleared.push_urls.is_empty());
    assert_eq!(
        f.out(&["remote", "get-url", "--push", "origin"]),
        "https://example.org/a.git"
    );
}

#[tokio::test]
async fn renaming_moves_the_tracking_references_and_refuses_a_taken_name() {
    let f = fixture().await;
    f.out(&["remote", "add", "origin", "https://example.org/a.git"]);
    f.out(&["remote", "add", "other", "https://example.org/o.git"]);
    let head = f.out(&["rev-parse", "HEAD"]);
    f.out(&["update-ref", "refs/remotes/origin/main", &head]);
    f.out(&["config", "branch.main.remote", "origin"]);
    f.out(&["config", "branch.main.merge", "refs/heads/main"]);
    assert_eq!(
        f.git
            .rename_remote(&f.info, "origin", "other")
            .await
            .unwrap_err(),
        RemoteError::AlreadyExists
    );
    assert_eq!(
        f.git
            .rename_remote(&f.info, "origin", "bad name")
            .await
            .unwrap_err(),
        RemoteError::InvalidName
    );
    let renamed = f
        .git
        .rename_remote(&f.info, "origin", "upstream")
        .await
        .unwrap();
    assert_eq!(renamed.name, "upstream");
    assert_eq!(f.out(&["rev-parse", "refs/remotes/upstream/main"]), head);
    assert!(f.out(&["for-each-ref", "refs/remotes/origin"]).is_empty());
    assert_eq!(f.out(&["config", "branch.main.remote"]), "upstream");
}

#[tokio::test]
async fn removal_reports_its_impact_and_only_goes_through_when_nothing_changed() {
    let f = fixture().await;
    f.out(&["remote", "add", "origin", "https://example.org/a.git"]);
    let head = f.out(&["rev-parse", "HEAD"]);
    f.out(&["update-ref", "refs/remotes/origin/main", &head]);
    f.out(&["update-ref", "refs/remotes/origin/dev", &head]);
    f.out(&["config", "branch.main.remote", "origin"]);
    f.out(&["config", "branch.main.merge", "refs/heads/main"]);
    let impact = f.git.remote_impact(&f.info, "origin").await.unwrap();
    assert_eq!(impact.tracking_refs, 2);
    assert_eq!(impact.upstream_of, ["main"]);
    assert_eq!(
        f.git
            .remove_remote(&f.info, "origin", &["https://other/".to_owned()])
            .await
            .unwrap_err(),
        RemoteError::Changed
    );
    assert_eq!(f.out(&["remote"]), "origin");
    let removed = f
        .git
        .remove_remote(&f.info, "origin", &["https://example.org/a.git".to_owned()])
        .await
        .unwrap();
    assert_eq!(removed.fetch_urls, ["https://example.org/a.git"]);
    assert!(f.out(&["remote"]).is_empty());
    assert!(f.out(&["for-each-ref", "refs/remotes"]).is_empty());
    assert_eq!(f.out(&["rev-parse", "HEAD"]), head);
    assert_eq!(f.out(&["symbolic-ref", "--short", "HEAD"]), "main");
}

#[tokio::test]
async fn a_change_applies_to_the_chosen_repository_only() {
    let a = fixture().await;
    let b = fixture().await;
    a.git
        .add_remote(&a.info, "origin", "https://example.org/a.git")
        .await
        .unwrap();
    b.git
        .add_remote(&b.info, "origin", "https://example.org/b.git")
        .await
        .unwrap();
    a.git
        .rename_remote(&a.info, "origin", "renamed")
        .await
        .unwrap();
    assert_eq!(b.out(&["remote"]), "origin");
    assert_eq!(
        b.out(&["config", "--get", "remote.origin.url"]),
        "https://example.org/b.git"
    );
}

#[test]
fn anything_that_can_carry_a_secret_in_a_url_is_detected_and_masked() {
    for secret in [
        "https://user:token@example.org/a.git",
        "https://ghp_abcdef123456@github.com/o/r.git",
        "http://token@example.org/a.git",
        "git://user@example.org/a.git",
        "ssh://user:password@example.org/a.git",
        "https://example.org/a.git?access_token=abc123",
        "https://example.org/a.git#abc123",
    ] {
        assert!(url_has_secret(secret), "{secret}");
        let masked = mask_url(secret);
        for leaked in ["token", "ghp_abcdef123456", "password", "abc123"] {
            assert!(!masked.contains(leaked), "{secret} -> {masked}");
        }
    }
    for plain in [
        "ssh://git@example.org/a.git",
        "git@example.org:a.git",
        "https://example.org/a@b:c.git",
        "https://example.org/a.git",
        "/srv/repo.git",
    ] {
        assert!(!url_has_secret(plain), "{plain}");
        assert_eq!(mask_url(plain), plain);
    }
    assert_eq!(
        mask_url("https://user:token@example.org/a.git"),
        "https://***@example.org/a.git"
    );
    assert_eq!(
        mask_url("https://example.org/a.git?access_token=abc123"),
        "https://example.org/a.git?***"
    );
}
