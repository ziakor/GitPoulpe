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
    isolated_git_with_agent(dir, None)
}

fn isolated_git_with_agent(dir: &Path, agent: Option<&Path>) -> Git {
    let script = dir.join("isolated-git.sh");
    let agent = agent.map_or_else(
        || "unset SSH_AUTH_SOCK".to_owned(),
        |socket| format!("export SSH_AUTH_SOCK='{}'", socket.display()),
    );
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nexport GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null\nunset EMAIL\n{agent}\nexec git \"$@\"\n"
        ),
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
        signature,
    } = outcome
    else {
        panic!("{outcome:?}")
    };
    assert_eq!(subject, "Add a second line");
    assert_eq!(before.as_deref(), Some(parent.as_str()));
    assert_eq!(signature, None);
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

fn amend_request(
    snapshot: &StatusSnapshot,
    info: &gitpoulpe_core::AmendInfo,
    message: &str,
    confirmed: bool,
) -> gitpoulpe_core::AmendRequest {
    gitpoulpe_core::AmendRequest {
        commit: request(snapshot, message, Some(profile(SigningPreference::None))),
        head: info.head.clone(),
        confirmed_published: confirmed,
    }
}

fn author_of_head(dir: &Path) -> String {
    String::from_utf8(run_git(
        dir,
        &["log", "-1", "--format=%an <%ae> | %cn <%ce> | %s"],
    ))
    .unwrap()
    .trim()
    .to_owned()
}

fn original_commit(repo: &Path) {
    write(repo, "a.txt", "one\n");
    commit_all(repo, "first version");
}

#[tokio::test]
async fn amending_the_message_then_the_content_replaces_the_tip_and_keeps_the_original_author() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path());
    let repo = init_repo();
    let dir = repo.path();
    original_commit(dir);
    let first = head(dir);

    let (info, snapshot) = open(&git, dir).await;
    let amend = git
        .amend_info(&info)
        .await
        .expect("there is a commit to amend");
    assert_eq!(amend.head, first);
    assert_eq!(amend.message, "first version");
    assert!(amend.published_on.is_empty());
    let outcome = git
        .amend(
            &info,
            &amend_request(&snapshot, &amend, "better message", false),
            &Cancel::new(),
        )
        .await
        .unwrap();
    let CommitOutcome::Created { before, oid, .. } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(
        before.as_deref(),
        Some(first.as_str()),
        "the previous tip is reported"
    );
    assert_ne!(oid, first);
    assert_eq!(
        run_git(dir, &["rev-list", "--count", "HEAD"]),
        b"1\n",
        "replaced, not added"
    );
    assert_eq!(
        author_of_head(dir),
        "t <t@example.com> | Ada Lovelace <ada@example.org> | better message",
        "the author is kept, the committer is the profile"
    );
    assert_eq!(
        run_git(dir, &["cat-file", "-t", &first]),
        b"commit\n",
        "the old commit still exists"
    );

    write(dir, "a.txt", "one\ntwo\n");
    run_git(dir, &["add", "a.txt"]);
    let (info, snapshot) = open(&git, dir).await;
    let amend = git.amend_info(&info).await.unwrap();
    let outcome = git
        .amend(
            &info,
            &amend_request(&snapshot, &amend, "better message", false),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome, CommitOutcome::Created { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        run_git(dir, &["show", "HEAD:a.txt"]),
        b"one\ntwo\n",
        "the content was amended"
    );
    assert_eq!(run_git(dir, &["rev-list", "--count", "HEAD"]), b"1\n");
}

#[tokio::test]
async fn a_published_commit_needs_an_explicit_confirmation_and_names_where_it_is_published() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path());
    let bare = tempfile::tempdir().unwrap();
    git_output(bare.path(), &["init", "-q", "--bare", "-b", "main"]);
    let repo = init_repo();
    let dir = repo.path();
    original_commit(dir);
    git_output(
        dir,
        &["remote", "add", "origin", &bare.path().to_string_lossy()],
    );
    assert!(
        git_output(dir, &["push", "-q", "origin", "main"])
            .status
            .success()
    );
    let (info, snapshot) = open(&git, dir).await;
    let amend = git.amend_info(&info).await.unwrap();
    assert_eq!(amend.published_on, vec!["origin/main".to_owned()]);
    let head_before = head(dir);

    let outcome = git
        .amend(
            &info,
            &amend_request(&snapshot, &amend, "rewritten", false),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert_eq!(outcome, CommitOutcome::Refused(CommitRefusal::Published));
    assert_eq!(
        head(dir),
        head_before,
        "nothing was rewritten without confirmation"
    );

    let outcome = git
        .amend(
            &info,
            &amend_request(&snapshot, &amend, "rewritten", true),
            &Cancel::new(),
        )
        .await
        .unwrap();
    let CommitOutcome::Created { before, .. } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(before.as_deref(), Some(head_before.as_str()));
    assert_eq!(
        run_git(dir, &["cat-file", "-t", &head_before]),
        b"commit\n",
        "the published commit is still reachable by its id"
    );
}

#[tokio::test]
async fn amending_is_refused_when_there_is_nothing_to_change_or_nothing_to_amend() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path());
    let repo = init_repo();
    let dir = repo.path();
    original_commit(dir);
    let (info, snapshot) = open(&git, dir).await;
    let amend = git.amend_info(&info).await.unwrap();
    let head_before = head(dir);

    for (message, expected) in [
        ("first version", CommitRefusal::NothingToAmend),
        ("  ", CommitRefusal::EmptyMessage),
    ] {
        let outcome = git
            .amend(
                &info,
                &amend_request(&snapshot, &amend, message, false),
                &Cancel::new(),
            )
            .await
            .unwrap();
        assert_eq!(outcome, CommitOutcome::Refused(expected), "{message:?}");
    }
    let mut stale = amend.clone();
    stale.head = "0".repeat(40);
    let outcome = git
        .amend(
            &info,
            &amend_request(&snapshot, &stale, "x", false),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert_eq!(outcome, CommitOutcome::Refused(CommitRefusal::Stale));
    assert_eq!(head(dir), head_before);

    let unborn = init_repo();
    let (info, snapshot) = open(&git, unborn.path()).await;
    assert!(git.amend_info(&info).await.is_none());
    let fake = gitpoulpe_core::AmendInfo {
        head: String::new(),
        message: String::new(),
        published_on: Vec::new(),
    };
    let outcome = git
        .amend(
            &info,
            &amend_request(&snapshot, &fake, "x", false),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert_eq!(outcome, CommitOutcome::Refused(CommitRefusal::NoCommit));
}

#[tokio::test]
async fn a_hook_that_refuses_an_amendment_leaves_the_tip_where_it_was_and_a_retry_works() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path());
    let repo = init_repo();
    let dir = repo.path();
    original_commit(dir);
    let commit_msg = hook(dir, "commit-msg", "echo 'subject too long' >&2\nexit 1");
    let (info, snapshot) = open(&git, dir).await;
    let amend = git.amend_info(&info).await.unwrap();
    let head_before = head(dir);

    let outcome = git
        .amend(
            &info,
            &amend_request(&snapshot, &amend, "a new subject", false),
            &Cancel::new(),
        )
        .await
        .unwrap();
    let CommitOutcome::Failed { kind, output } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(kind, CommitFailureKind::Hook);
    assert!(output.contains("subject too long"), "{output}");
    assert_eq!(
        head(dir),
        head_before,
        "the previous tip is untouched and still the tip"
    );

    std::fs::remove_file(&commit_msg).unwrap();
    let outcome = git
        .amend(
            &info,
            &amend_request(&snapshot, &amend, "a new subject", false),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome, CommitOutcome::Created { .. }),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn amending_a_merge_commit_keeps_both_parents() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path());
    let repo = init_repo();
    let dir = repo.path();
    original_commit(dir);
    run_git(dir, &["checkout", "-q", "-b", "side"]);
    write(dir, "side.txt", "s\n");
    commit_all(dir, "side work");
    run_git(dir, &["checkout", "-q", "main"]);
    write(dir, "main.txt", "m\n");
    commit_all(dir, "main work");
    run_git(dir, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    let parents_before = run_git(dir, &["rev-list", "--parents", "-1", "HEAD"]);
    let (info, snapshot) = open(&git, dir).await;
    let amend = git.amend_info(&info).await.unwrap();
    let outcome = git
        .amend(
            &info,
            &amend_request(&snapshot, &amend, "merge side branch", false),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome, CommitOutcome::Created { .. }),
        "{outcome:?}"
    );
    let after = String::from_utf8(run_git(dir, &["rev-list", "--parents", "-1", "HEAD"])).unwrap();
    let before = String::from_utf8(parents_before).unwrap();
    assert_eq!(
        after.split_whitespace().skip(1).collect::<Vec<_>>(),
        before.split_whitespace().skip(1).collect::<Vec<_>>(),
        "the parents of the merge are unchanged"
    );
}

struct Agent {
    child: std::process::Child,
    socket: PathBuf,
}

impl Agent {
    fn start(dir: &Path) -> Self {
        let socket = dir.join("agent.sock");
        let child = std::process::Command::new("ssh-agent")
            .args(["-D", "-a"])
            .arg(&socket)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("ssh-agent starts");
        for _ in 0..200 {
            if socket.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(socket.exists(), "the agent socket appeared");
        Self { child, socket }
    }

    fn add(&self, private_key: &Path) {
        let status = std::process::Command::new("ssh-add")
            .env("SSH_AUTH_SOCK", &self.socket)
            .arg(private_key)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "the key was added to the agent");
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static VERIFY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn new_ssh_key(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    let private = dir.join(name);
    let status = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f"])
        .arg(&private)
        .status()
        .unwrap();
    assert!(status.success());
    let public = dir.join(format!("{name}.pub"));
    (private, public)
}

fn verify_with(
    dir: &Path,
    principal: &str,
    public_key: &Path,
    work: &Path,
) -> std::process::Output {
    let text = std::fs::read_to_string(public_key).unwrap();
    let mut fields = text.split_whitespace();
    let allowed = work.join(format!("allowed-{principal}"));
    std::fs::write(
        &allowed,
        format!(
            "{principal} {} {}\n",
            fields.next().unwrap(),
            fields.next().unwrap()
        ),
    )
    .unwrap();
    git_output(
        dir,
        &[
            "-c",
            &format!("gpg.ssh.allowedSignersFile={}", allowed.display()),
            "verify-commit",
            "HEAD",
        ],
    )
}

#[tokio::test]
async fn a_commit_signed_with_an_ssh_key_from_the_agent_is_verifiable_by_git() {
    let _serial = VERIFY_LOCK.lock().await;
    let tools = tempfile::tempdir().unwrap();
    let (private, public) = new_ssh_key(tools.path(), "id_test");
    let (_, other_public) = new_ssh_key(tools.path(), "id_other");
    let agent = Agent::start(tools.path());
    agent.add(&private);
    let git = isolated_git_with_agent(tools.path(), Some(&agent.socket));
    let repo = repo_with_staged_change();
    let dir = repo.path();
    let (info, snapshot) = open(&git, dir).await;
    let ssh = profile(SigningPreference::Ssh {
        public_key: public.clone(),
    });
    let private_before = std::fs::read(&private).unwrap();

    let outcome = git
        .commit(
            &info,
            &request(&snapshot, "Signed change", Some(ssh)),
            &Cancel::new(),
        )
        .await
        .unwrap();
    let CommitOutcome::Created { signature, .. } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(
        signature,
        Some(gitpoulpe_core::Signature {
            kind: gitpoulpe_core::SignatureKind::Ssh,
            verified: true
        }),
        "the result reports an SSH signature verified against the profile key"
    );

    let good = verify_with(dir, "ada@example.org", &public, tools.path());
    assert!(
        good.status.success(),
        "Git itself verifies the signature: {}",
        String::from_utf8_lossy(&good.stderr)
    );
    assert!(
        String::from_utf8_lossy(&good.stderr).contains("Good"),
        "{}",
        String::from_utf8_lossy(&good.stderr)
    );
    let wrong = verify_with(dir, "ada@example.org", &other_public, tools.path());
    assert!(
        !wrong.status.success(),
        "another key does not verify the signature"
    );
    assert!(
        String::from_utf8_lossy(&run_git(dir, &["cat-file", "commit", "HEAD"]))
            .contains("BEGIN SSH SIGNATURE"),
        "the commit object carries the SSH signature"
    );

    assert_eq!(
        std::fs::read(&private).unwrap(),
        private_before,
        "the private key was left alone"
    );
    let leftovers = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(&format!("gitpoulpe-verify-{}", std::process::id()))
        })
        .count();
    assert_eq!(
        leftovers, 0,
        "the temporary verification files were removed"
    );
    let in_repo = std::process::Command::new("grep")
        .args(["-rl", "PRIVATE KEY"])
        .arg(dir.join(".git"))
        .output()
        .unwrap();
    assert!(
        in_repo.stdout.is_empty(),
        "no private key material in the repository"
    );
}

#[tokio::test]
async fn without_a_usable_agent_the_signature_fails_cleanly_and_nothing_is_committed() {
    let tools = tempfile::tempdir().unwrap();
    let (private, public) = new_ssh_key(tools.path(), "id_test");
    std::fs::remove_file(&private).unwrap();
    let git = isolated_git(tools.path());
    let repo = repo_with_staged_change();
    let dir = repo.path();
    let (info, snapshot) = open(&git, dir).await;
    let head_before = head(dir);
    let index_before = run_git(dir, &["ls-files", "--stage", "-z"]);
    let ssh = profile(SigningPreference::Ssh { public_key: public });

    let outcome = git
        .commit(
            &info,
            &request(&snapshot, "Cannot be signed", Some(ssh)),
            &Cancel::new(),
        )
        .await
        .unwrap();
    let CommitOutcome::Failed { kind, output } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(kind, CommitFailureKind::SigningUnavailable);
    assert!(!output.trim().is_empty(), "the diagnostic is kept");
    assert_eq!(head(dir), head_before);
    assert_eq!(run_git(dir, &["ls-files", "--stage", "-z"]), index_before);
}

#[tokio::test]
async fn an_amended_commit_is_signed_again() {
    let _serial = VERIFY_LOCK.lock().await;
    let tools = tempfile::tempdir().unwrap();
    let (private, public) = new_ssh_key(tools.path(), "id_test");
    let agent = Agent::start(tools.path());
    agent.add(&private);
    let git = isolated_git_with_agent(tools.path(), Some(&agent.socket));
    let repo = init_repo();
    let dir = repo.path();
    original_commit(dir);
    let (info, snapshot) = open(&git, dir).await;
    let amend = git.amend_info(&info).await.unwrap();
    let ssh = profile(SigningPreference::Ssh {
        public_key: public.clone(),
    });
    let request = gitpoulpe_core::AmendRequest {
        commit: request(&snapshot, "reworded and signed", Some(ssh)),
        head: amend.head.clone(),
        confirmed_published: false,
    };
    let outcome = git.amend(&info, &request, &Cancel::new()).await.unwrap();
    let CommitOutcome::Created { signature, .. } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(signature.map(|s| s.verified), Some(true));
    assert!(
        verify_with(dir, "ada@example.org", &public, tools.path())
            .status
            .success()
    );
}

#[tokio::test]
async fn the_signature_check_accepts_only_the_key_that_signed() {
    let _serial = VERIFY_LOCK.lock().await;
    let tools = tempfile::tempdir().unwrap();
    let (private, public) = new_ssh_key(tools.path(), "id_signer");
    let (_, other_public) = new_ssh_key(tools.path(), "id_other");
    let git = isolated_git(tools.path());
    let repo = repo_with_staged_change();
    let dir = repo.path();
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "gpg.format=ssh",
            "-c",
            &format!("user.signingkey={}", private.display()),
            "-c",
            "commit.gpgsign=true",
            "commit",
            "-q",
            "-m",
            "signed with the key file",
        ])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.org")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.org")
        .status()
        .unwrap();
    assert!(status.success());
    let (info, _) = open(&git, dir).await;

    assert!(
        git.verify_ssh_signature(&info, &public, "ada@example.org")
            .await
    );
    assert!(
        !git.verify_ssh_signature(&info, &other_public, "ada@example.org")
            .await,
        "another key does not match"
    );
    assert!(
        !git.verify_ssh_signature(&info, &tools.path().join("absent.pub"), "ada@example.org")
            .await
    );
}
