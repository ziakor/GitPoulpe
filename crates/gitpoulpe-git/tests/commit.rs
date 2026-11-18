mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{commit_all, git as run_git, git_output, init_repo, write};
use gitpoulpe_core::{
    CommitFailureKind, CommitOutcome, CommitRefusal, CommitRequest, Generation, Profile,
    SessionInfo, SigningPreference, StatusSnapshot, WriteError,
};
use gitpoulpe_git::{Cancel, Git};

fn isolated_git(dir: &Path) -> Git {
    let script = dir.join("isolated-git.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nexport GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null\nunset EMAIL\nexec git \"$@\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    Git::with_program(script)
}

fn profile(signing: SigningPreference) -> Profile {
    Profile {
        id: "p1".into(),
        label: "Test".into(),
        user_name: "Ada Lovelace".into(),
        user_email: "ada@example.org".into(),
        signing,
    }
}

async fn open(git: &Git, dir: &Path) -> (SessionInfo, StatusSnapshot) {
    let info = git.open(dir, &Cancel::new()).await.unwrap();
    let snapshot = git
        .read_status(&info, Generation::default().next(), &Cancel::new())
        .await
        .unwrap();
    (info, snapshot)
}

fn request(snapshot: &StatusSnapshot, message: &str, profile: Option<Profile>) -> CommitRequest {
    CommitRequest {
        message: message.to_owned(),
        staged: snapshot
            .entries
            .iter()
            .filter(|e| e.is_staged())
            .map(|e| {
                (
                    e.path.clone(),
                    snapshot.index_ids.get(&e.path).cloned().unwrap_or_default(),
                )
            })
            .collect(),
        profile,
    }
}

fn head(dir: &Path) -> String {
    String::from_utf8(run_git(dir, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned()
}

fn hook(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(".git/hooks").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn repo_with_staged_change() -> tempfile::TempDir {
    let repo = init_repo();
    write(repo.path(), "a.txt", "one\n");
    write(repo.path(), "b.txt", "bee\n");
    commit_all(repo.path(), "base");
    write(repo.path(), "a.txt", "one\ntwo\n");
    run_git(repo.path(), &["add", "a.txt"]);
    write(repo.path(), "b.txt", "bee local edit\n");
    repo
}

#[tokio::test]
async fn the_commit_holds_exactly_the_staged_content_with_the_announced_message_and_identity() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path());
    let repo = repo_with_staged_change();
    let dir = repo.path();
    let (info, snapshot) = open(&git, dir).await;
    let tree_before = run_git(dir, &["write-tree"]);
    let parent = head(dir);
    let message = "Add a second line\n\nLonger explanation, with ünïcode.\n";

    let outcome = git
        .commit(
            &info,
            &request(&snapshot, message, Some(profile(SigningPreference::None))),
            &Cancel::new(),
        )
        .await
        .unwrap();
    let CommitOutcome::Created {
        oid,
        subject,
        before,
        signed,
    } = outcome
    else {
        panic!("{outcome:?}")
    };
    assert_eq!(subject, "Add a second line");
    assert_eq!(before.as_deref(), Some(parent.as_str()));
    assert!(!signed);
    assert_eq!(head(dir), oid);
    assert_eq!(
        tree_hash(&run_git(dir, &["rev-parse", "HEAD^{tree}"])),
        tree_hash(&tree_before),
        "the commit tree is the index tree that was validated"
    );
    assert_eq!(
        String::from_utf8(run_git(dir, &["log", "-1", "--format=%B"]))
            .unwrap()
            .trim_end(),
        message.trim_end()
    );
    assert_eq!(
        String::from_utf8(run_git(dir, &["log", "-1", "--format=%an <%ae>|%cn <%ce>"]))
            .unwrap()
            .trim(),
        "Ada Lovelace <ada@example.org>|Ada Lovelace <ada@example.org>"
    );
    assert_eq!(
        std::fs::read(dir.join("b.txt")).unwrap(),
        b"bee local edit\n"
    );
    assert!(
        String::from_utf8(run_git(dir, &["status", "--porcelain"]))
            .unwrap()
            .contains(" M b.txt"),
        "the unstaged edit is still there"
    );
}

fn tree_hash(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

#[tokio::test]
async fn the_first_commit_of_a_repository_works() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path());
    let repo = init_repo();
    write(repo.path(), "first.txt", "1\n");
    run_git(repo.path(), &["add", "first.txt"]);
    let (info, snapshot) = open(&git, repo.path()).await;
    assert!(!info.has_commits);
    let outcome = git
        .commit(
            &info,
            &request(
                &snapshot,
                "Initial commit",
                Some(profile(SigningPreference::None)),
            ),
            &Cancel::new(),
        )
        .await
        .unwrap();
    let CommitOutcome::Created { before, oid, .. } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(before, None);
    assert_eq!(head(repo.path()), oid);
    assert_eq!(
        run_git(repo.path(), &["rev-list", "--count", "HEAD"]),
        b"1\n"
    );
}

#[tokio::test]
async fn refusals_leave_head_and_index_untouched() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path());
    let repo = repo_with_staged_change();
    let dir = repo.path();
    let (info, snapshot) = open(&git, dir).await;
    let head_before = head(dir);
    let index_before = run_git(dir, &["ls-files", "--stage", "-z"]);
    let same = |what: &str| {
        assert_eq!(head(dir), head_before, "{what}: HEAD");
        assert_eq!(
            run_git(dir, &["ls-files", "--stage", "-z"]),
            index_before,
            "{what}: index"
        );
    };
    let p = Some(profile(SigningPreference::None));

    for blank in ["", "   \n\t\n"] {
        let outcome = git
            .commit(&info, &request(&snapshot, blank, p.clone()), &Cancel::new())
            .await
            .unwrap();
        assert_eq!(outcome, CommitOutcome::Refused(CommitRefusal::EmptyMessage));
    }
    same("empty message");

    let mut stale = request(&snapshot, "msg", p.clone());
    stale
        .staged
        .push((PathBuf::from("ghost.txt"), "100644:abc".into()));
    let outcome = git.commit(&info, &stale, &Cancel::new()).await.unwrap();
    assert_eq!(outcome, CommitOutcome::Refused(CommitRefusal::Stale));
    let mut other = request(&snapshot, "msg", p.clone());
    other.staged[0].1 = "100644:0000".into();
    assert_eq!(
        git.commit(&info, &other, &Cancel::new()).await.unwrap(),
        CommitOutcome::Refused(CommitRefusal::Stale)
    );
    same("stale index");

    run_git(dir, &["reset", "-q"]);
    let (info, snapshot) = open(&git, dir).await;
    let empty = git
        .commit(&info, &request(&snapshot, "msg", p), &Cancel::new())
        .await
        .unwrap();
    assert_eq!(empty, CommitOutcome::Refused(CommitRefusal::EmptyIndex));
    assert_eq!(head(dir), head_before);
}

#[tokio::test]
async fn pending_conflicts_are_refused() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path());
    let repo = init_repo();
    let dir = repo.path();
    write(dir, "a.txt", "base\n");
    commit_all(dir, "base");
    run_git(dir, &["checkout", "-q", "-b", "side"]);
    write(dir, "a.txt", "side\n");
    commit_all(dir, "side");
    run_git(dir, &["checkout", "-q", "main"]);
    write(dir, "a.txt", "main\n");
    commit_all(dir, "main");
    let _ = git_output(dir, &["merge", "side"]);
    let (info, snapshot) = open(&git, dir).await;
    let outcome = git
        .commit(
            &info,
            &request(&snapshot, "msg", Some(profile(SigningPreference::None))),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert_eq!(outcome, CommitOutcome::Refused(CommitRefusal::Conflicts));
}

#[tokio::test]
async fn an_unknown_identity_stops_before_anything_is_written() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path());
    let repo = repo_with_staged_change();
    let dir = repo.path();
    git_output(dir, &["config", "user.useConfigOnly", "true"]);
    let (info, snapshot) = open(&git, dir).await;
    let head_before = head(dir);
    let outcome = git
        .commit(&info, &request(&snapshot, "msg", None), &Cancel::new())
        .await
        .unwrap();
    let CommitOutcome::Failed { kind, output } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(kind, CommitFailureKind::IdentityUnknown);
    assert!(!output.is_empty());
    assert_eq!(head(dir), head_before);
    assert!(
        String::from_utf8(run_git(dir, &["diff", "--cached", "--name-only"]))
            .unwrap()
            .contains("a.txt"),
        "the staged work is still staged"
    );
}

#[tokio::test]
async fn a_refusing_hook_keeps_its_output_and_the_commit_can_be_retried_once_fixed() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path());
    let repo = repo_with_staged_change();
    let dir = repo.path();
    let pre_commit = hook(
        dir,
        "pre-commit",
        "echo 'lint failed: a.txt has trailing spaces' >&2\nexit 1",
    );
    let (info, snapshot) = open(&git, dir).await;
    let p = Some(profile(SigningPreference::None));
    let head_before = head(dir);
    let index_before = run_git(dir, &["ls-files", "--stage", "-z"]);

    let outcome = git
        .commit(&info, &request(&snapshot, "msg", p.clone()), &Cancel::new())
        .await
        .unwrap();
    let CommitOutcome::Failed { kind, output } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(kind, CommitFailureKind::Hook);
    assert!(
        output.contains("lint failed: a.txt has trailing spaces"),
        "{output}"
    );
    assert_eq!(head(dir), head_before);
    assert_eq!(run_git(dir, &["ls-files", "--stage", "-z"]), index_before);

    std::fs::remove_file(&pre_commit).unwrap();
    let (info, snapshot) = open(&git, dir).await;
    let retry = git
        .commit(&info, &request(&snapshot, "msg", p.clone()), &Cancel::new())
        .await
        .unwrap();
    assert!(matches!(retry, CommitOutcome::Created { .. }), "{retry:?}");

    write(dir, "c.txt", "c\n");
    run_git(dir, &["add", "c.txt"]);
    hook(
        dir,
        "commit-msg",
        "echo 'message must start with a ticket id' >&2\nexit 1",
    );
    let (info, snapshot) = open(&git, dir).await;
    let outcome = git
        .commit(&info, &request(&snapshot, "no ticket", p), &Cancel::new())
        .await
        .unwrap();
    let CommitOutcome::Failed { kind, output } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(kind, CommitFailureKind::Hook);
    assert!(
        output.contains("message must start with a ticket id"),
        "{output}"
    );
}

#[tokio::test]
async fn a_signature_that_cannot_be_produced_is_reported_and_nothing_is_committed() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path());
    let repo = repo_with_staged_change();
    let dir = repo.path();
    let (info, snapshot) = open(&git, dir).await;
    let head_before = head(dir);
    let missing = profile(SigningPreference::Ssh {
        public_key: tools.path().join("absent.pub"),
    });
    let outcome = git
        .commit(
            &info,
            &request(&snapshot, "msg", Some(missing)),
            &Cancel::new(),
        )
        .await
        .unwrap();
    let CommitOutcome::Failed { kind, output } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(kind, CommitFailureKind::SigningUnavailable);
    assert!(output.contains("introuvable"), "{output}");

    let unknown_gpg = profile(SigningPreference::Gpg {
        key_id: "DEADBEEFDEADBEEF".into(),
    });
    let outcome = git
        .commit(
            &info,
            &request(&snapshot, "msg", Some(unknown_gpg)),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        CommitOutcome::Failed {
            kind: CommitFailureKind::SigningUnavailable,
            ..
        }
    ));
    assert_eq!(head(dir), head_before);
}

#[tokio::test]
async fn a_locked_index_is_reported_and_the_lock_stays() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path());
    let repo = repo_with_staged_change();
    let dir = repo.path();
    let (info, snapshot) = open(&git, dir).await;
    let lock = dir.join(".git/index.lock");
    std::fs::write(&lock, b"").unwrap();
    let error = git
        .commit(
            &info,
            &request(&snapshot, "msg", Some(profile(SigningPreference::None))),
            &Cancel::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, WriteError::IndexLocked(_)), "{error:?}");
    assert!(lock.exists());
}
