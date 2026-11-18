mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::{commit_all, git as run_git, git_output, init_repo, write};
use gitpoulpe_core::{Profile, SessionInfo, SigningPreference};
use gitpoulpe_git::{Cancel, Git, TagError};

struct Fixture {
    repo: tempfile::TempDir,
    git: Git,
    info: SessionInfo,
    first: String,
    second: String,
}

async fn fixture() -> Fixture {
    let repo = init_repo();
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_AUTHOR_NAME", "Config Person")
        .with_env("GIT_AUTHOR_EMAIL", "config@example.org")
        .with_env("GIT_COMMITTER_NAME", "Config Person")
        .with_env("GIT_COMMITTER_EMAIL", "config@example.org");
    write(repo.path(), "a.txt", "one\n");
    commit_all(repo.path(), "first");
    let first = String::from_utf8_lossy(&run_git(repo.path(), &["rev-parse", "HEAD"]))
        .trim()
        .to_owned();
    write(repo.path(), "b.txt", "two\n");
    commit_all(repo.path(), "second");
    let second = String::from_utf8_lossy(&run_git(repo.path(), &["rev-parse", "HEAD"]))
        .trim()
        .to_owned();
    let info = git.open(repo.path(), &Cancel::new()).await.unwrap();
    Fixture {
        repo,
        git,
        info,
        first,
        second,
    }
}

impl Fixture {
    fn path(&self) -> &Path {
        self.repo.path()
    }
    fn out(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&run_git(self.path(), args))
            .trim()
            .to_owned()
    }
}

fn ada() -> Profile {
    Profile {
        id: "p".into(),
        label: "Ada".into(),
        user_name: "Ada Lovelace".into(),
        user_email: "ada@example.org".into(),
        signing: SigningPreference::None,
    }
}

#[tokio::test]
async fn light_and_annotated_tags_point_at_the_chosen_commit_and_their_details_are_readable() {
    let f = fixture().await;
    let light = f
        .git
        .create_tag(&f.info, "v0.1", Some(&f.first), None, None)
        .await
        .unwrap();
    assert!(!light.annotated);
    assert_eq!(light.object, f.first, "a light tag is the commit itself");
    assert_eq!(light.target.as_deref(), Some(f.first.as_str()));
    assert_eq!(f.out(&["rev-parse", "refs/tags/v0.1"]), f.first);
    assert_eq!(f.out(&["cat-file", "-t", "refs/tags/v0.1"]), "commit");

    let message = "Release 1.0\n\n# not a comment\nSecond paragraph — with accents é\n";
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let annotated = f
        .git
        .create_tag(&f.info, "v1.0", None, Some(message), Some(&ada()))
        .await
        .unwrap();
    assert!(annotated.annotated);
    assert_eq!(
        annotated.target.as_deref(),
        Some(f.second.as_str()),
        "HEAD by default"
    );
    assert_eq!(f.out(&["cat-file", "-t", "refs/tags/v1.0"]), "tag");
    assert_eq!(f.out(&["rev-parse", "refs/tags/v1.0^{commit}"]), f.second);
    assert_eq!(annotated.object, f.out(&["rev-parse", "refs/tags/v1.0"]));
    assert_ne!(annotated.object, f.second);
    assert_eq!(annotated.tagger.as_deref(), Some("Ada Lovelace"));
    assert_eq!(annotated.tagger_email.as_deref(), Some("ada@example.org"));
    assert!(annotated.timestamp.unwrap() >= before - 2);
    assert_eq!(annotated.subject, "Release 1.0");
    assert_eq!(annotated.message.as_deref(), Some(message.trim_end()));

    let raw = f.out(&["cat-file", "tag", "refs/tags/v1.0"]);
    assert!(
        raw.contains("tagger Ada Lovelace <ada@example.org>"),
        "{raw}"
    );
    assert!(
        raw.contains("# not a comment") && raw.contains("accents é"),
        "{raw}"
    );

    let listed = f.git.list_tags(&f.info).await.unwrap();
    assert_eq!(
        listed.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        f.out(&["tag", "--list"]).lines().collect::<Vec<_>>()
    );
    assert_eq!(listed[0], light);
    assert_eq!(listed[1], annotated);
}

#[tokio::test]
async fn without_a_profile_the_configured_identity_signs_and_an_unknown_one_creates_nothing() {
    let f = fixture().await;
    let tag = f
        .git
        .create_tag(&f.info, "cfg", None, Some("by config"), None)
        .await
        .unwrap();
    assert_eq!(tag.tagger.as_deref(), Some("Config Person"));

    let tools = tempfile::tempdir().unwrap();
    let script = tools.path().join("no-identity.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nexport GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null\nunset EMAIL GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL\nexec git \"$@\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let bare = Git::with_program(&script);
    git_output(f.path(), &["config", "user.useConfigOnly", "true"]);
    let before = f.out(&["tag", "--list"]);
    let error = bare
        .create_tag(&f.info, "nobody", None, Some("x"), None)
        .await
        .unwrap_err();
    assert!(matches!(error, TagError::IdentityUnknown(_)), "{error:?}");
    assert_eq!(f.out(&["tag", "--list"]), before);
}

#[tokio::test]
async fn invalid_existing_and_unusable_requests_are_refused_and_nothing_is_replaced() {
    let f = fixture().await;
    let kept = f
        .git
        .create_tag(&f.info, "keep", Some(&f.first), Some("original"), None)
        .await
        .unwrap();
    for name in [
        "", "a b", "x..y", "-x", "--force", "@", "HEAD", "x.lock", "a~b", "a^b", "a:b", "a?b",
        "a*b", "a[b", "a\nb", "end/", ".dot", "a//b", "a\\b",
    ] {
        assert_eq!(
            f.git.create_tag(&f.info, name, None, None, None).await,
            Err(TagError::InvalidName),
            "{name:?}"
        );
    }
    assert_eq!(
        f.git
            .create_tag(&f.info, "keep", Some(&f.second), None, None)
            .await,
        Err(TagError::AlreadyExists)
    );
    assert_eq!(
        f.git
            .create_tag(&f.info, "keep", Some(&f.second), Some("other"), None)
            .await,
        Err(TagError::AlreadyExists)
    );
    assert_eq!(
        f.out(&["rev-parse", "refs/tags/keep"]),
        kept.object,
        "never replaced"
    );

    assert_eq!(
        f.git
            .create_tag(&f.info, "empty", None, Some("  \n "), None)
            .await,
        Err(TagError::EmptyMessage)
    );
    let tree = f.out(&["rev-parse", "HEAD^{tree}"]);
    for target in [
        "zz",
        "--all",
        "abc",
        "0123456789abcdef0123456789abcdef01234567",
        &tree,
    ] {
        assert_eq!(
            f.git
                .create_tag(&f.info, "t", Some(target), None, None)
                .await,
            Err(TagError::InvalidTarget),
            "{target:?}"
        );
    }
    assert_eq!(f.out(&["tag", "--list"]), "keep");
}

#[tokio::test]
async fn deleting_a_tag_removes_only_that_tag_and_leaves_every_branch_alone() {
    let f = fixture().await;
    f.git
        .create_tag(&f.info, "gone", Some(&f.first), Some("bye"), None)
        .await
        .unwrap();
    f.git
        .create_tag(&f.info, "stay", None, None, None)
        .await
        .unwrap();
    run_git(f.path(), &["branch", "side", &f.first]);
    let refs = |f: &Fixture| {
        f.out(&[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads",
        ])
    };
    let branches = refs(&f);
    let head = f.out(&["rev-parse", "HEAD"]);
    let object = f.out(&["rev-parse", "refs/tags/gone"]);

    assert_eq!(
        f.git.delete_tag(&f.info, "gone", &f.first).await,
        Err(TagError::Changed),
        "a different expected object does not authorise the deletion"
    );
    assert_eq!(f.out(&["tag", "--list"]), "gone\nstay");

    assert_eq!(
        f.git.delete_tag(&f.info, "gone", &object).await,
        Ok(object.clone())
    );
    assert_eq!(f.out(&["tag", "--list"]), "stay");
    assert_eq!(refs(&f), branches);
    assert_eq!(f.out(&["rev-parse", "HEAD"]), head);
    assert_eq!(
        f.git.delete_tag(&f.info, "gone", &object).await,
        Err(TagError::NoSuchTag)
    );

    run_git(f.path(), &["update-ref", "refs/tags/gone", &object]);
    assert_eq!(
        f.out(&["cat-file", "-t", "refs/tags/gone"]),
        "tag",
        "the annotation is recoverable"
    );
    assert_eq!(f.out(&["rev-parse", "refs/tags/gone^{commit}"]), f.first);
}

#[tokio::test]
async fn tags_on_other_objects_and_nested_tags_are_read_without_guessing() {
    let f = fixture().await;
    let tree = f.out(&["rev-parse", "HEAD^{tree}"]);
    run_git(f.path(), &["tag", "on-tree", &tree]);
    run_git(f.path(), &["tag", "-a", "-m", "inner", "inner", &f.first]);
    run_git(f.path(), &["tag", "-a", "-m", "outer", "outer", "inner"]);
    let list = f.git.list_tags(&f.info).await.unwrap();
    let get = |n: &str| list.iter().find(|t| t.name == n).unwrap();
    assert_eq!(get("on-tree").target, None);
    assert!(!get("on-tree").annotated);
    assert!(get("outer").annotated);
    assert_eq!(
        get("outer").target.as_deref(),
        Some(f.first.as_str()),
        "a tag of a tag is followed to its commit"
    );
    assert_eq!(get("inner").target.as_deref(), Some(f.first.as_str()));
}
