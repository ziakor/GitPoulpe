mod common;

use std::path::{Path, PathBuf};

use common::{commit_all, git as run_git, git_output, init_repo, write};
use gitpoulpe_core::{
    ConflictKind, EmptyPolicy, OperationKind, Profile, SequenceKind, SequenceOutcome, SessionInfo,
    SigningPreference,
};
use gitpoulpe_git::{Cancel, ConflictSide, Git, SequenceError, SequenceRequest};

struct Env {
    repo: tempfile::TempDir,
    git: Git,
    info: SessionInfo,
}

impl Env {
    fn path(&self) -> &Path {
        self.repo.path()
    }
    fn out(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&run_git(self.path(), args))
            .trim()
            .to_owned()
    }
    fn head(&self) -> String {
        self.out(&["rev-parse", "HEAD"])
    }
    fn oid(&self, rev: &str) -> String {
        self.out(&["rev-parse", rev])
    }
    async fn run(
        &self,
        kind: SequenceKind,
        revs: &[&str],
        mainline: Option<u32>,
        empty: EmptyPolicy,
    ) -> Result<SequenceOutcome, SequenceError> {
        let oids: Vec<String> = revs.iter().map(|r| self.oid(r)).collect();
        self.git
            .run_sequence(
                &self.info,
                &SequenceRequest {
                    kind,
                    oids: &oids,
                    mainline,
                    empty,
                    expected_head: &self.head(),
                },
                None,
            )
            .await
    }
}

async fn env_with(setup: impl FnOnce(&Path)) -> Env {
    let repo = init_repo();
    setup(repo.path());
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com");
    let info = git.open(repo.path(), &Cancel::new()).await.unwrap();
    Env { repo, git, info }
}

fn side_branch(p: &Path) {
    write(p, "base.txt", "base\n");
    commit_all(p, "base");
    run_git(p, &["checkout", "-q", "-b", "side"]);
    for (n, content) in [("one", "1\n"), ("two", "2\n"), ("three", "3\n")] {
        write(p, &format!("{n}.txt"), content);
        commit_all(p, &format!("side {n}"));
    }
    run_git(p, &["checkout", "-q", "main"]);
}

#[tokio::test]
async fn the_preview_names_the_commits_in_the_announced_order_and_flags_merges() {
    let e = env_with(side_branch).await;
    let oids: Vec<String> = ["side~1", "side", "side~2"]
        .iter()
        .map(|r| e.oid(r))
        .collect();
    let preview = e
        .git
        .sequence_preview(&e.info, SequenceKind::CherryPick, &oids)
        .await
        .unwrap();
    let subjects: Vec<&str> = preview.commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["side two", "side three", "side one"]);
    assert_eq!(preview.merges, 0);
    assert_eq!(preview.branch.as_deref(), Some("main"));
    assert_eq!(preview.head, e.head());
}

#[tokio::test]
async fn selected_commits_are_applied_in_the_announced_order() {
    let e = env_with(side_branch).await;
    let outcome = e
        .run(
            SequenceKind::CherryPick,
            &["side", "side~2", "side~1"],
            None,
            EmptyPolicy::Drop,
        )
        .await
        .unwrap();
    let SequenceOutcome::Done { created, head } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(created, 3);
    assert_eq!(head, e.head());
    let log = e.out(&["log", "--format=%s", "-3"]);
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        ["side two", "side one", "side three"]
    );
    for f in ["one.txt", "two.txt", "three.txt"] {
        assert!(e.path().join(f).exists(), "{f}");
    }
}

#[tokio::test]
async fn an_intermediate_conflict_keeps_the_remaining_list_and_resumes_after_resolution() {
    let e = env_with(|p| {
        write(p, "a.txt", "base\n");
        commit_all(p, "base");
        run_git(p, &["checkout", "-q", "-b", "side"]);
        write(p, "first.txt", "first\n");
        commit_all(p, "side first");
        write(p, "a.txt", "side edit\n");
        commit_all(p, "side conflicting");
        write(p, "last.txt", "last\n");
        commit_all(p, "side last");
        run_git(p, &["checkout", "-q", "main"]);
        write(p, "a.txt", "main edit\n");
        commit_all(p, "main edit");
    })
    .await;
    let outcome = e
        .run(
            SequenceKind::CherryPick,
            &["side~2", "side~1", "side"],
            None,
            EmptyPolicy::Drop,
        )
        .await
        .unwrap();
    assert_eq!(
        outcome,
        SequenceOutcome::Conflicts {
            files: vec![PathBuf::from("a.txt")]
        }
    );
    assert!(e.path().join("first.txt").exists());
    assert!(!e.path().join("last.txt").exists());
    let op = e.git.operation_state(&e.info).await.unwrap().unwrap();
    assert_eq!(op.kind, OperationKind::CherryPick);
    assert!(
        op.theirs_label.contains("side conflicting"),
        "{}",
        op.theirs_label
    );
    let files = e.git.list_conflicts(&e.info).await.unwrap();
    assert_eq!(files[0].kind, ConflictKind::BothModified);
    e.git
        .resolve_with_content(&e.info, Path::new("a.txt"), b"merged edit\n", false)
        .await
        .unwrap();
    let done = e
        .git
        .continue_operation(&e.info, None, &[PathBuf::from("a.txt")])
        .await
        .unwrap();
    assert!(done.finished, "{}", done.message);
    assert!(
        e.path().join("last.txt").exists(),
        "the remaining commit was applied"
    );
    assert_eq!(
        e.out(&["log", "--format=%s", "-3"])
            .lines()
            .collect::<Vec<_>>(),
        ["side last", "side conflicting", "side first"]
    );
}

#[tokio::test]
async fn aborting_a_sequence_restores_the_branch_as_it_was() {
    let e = env_with(|p| {
        write(p, "a.txt", "base\n");
        commit_all(p, "base");
        run_git(p, &["checkout", "-q", "-b", "side"]);
        write(p, "first.txt", "first\n");
        commit_all(p, "side first");
        write(p, "a.txt", "side edit\n");
        commit_all(p, "side conflicting");
        run_git(p, &["checkout", "-q", "main"]);
        write(p, "a.txt", "main edit\n");
        commit_all(p, "main edit");
    })
    .await;
    let before = e.head();
    e.run(
        SequenceKind::CherryPick,
        &["side~1", "side"],
        None,
        EmptyPolicy::Drop,
    )
    .await
    .unwrap();
    assert_ne!(e.head(), before);
    e.git
        .abort_operation(&e.info, &[PathBuf::from("a.txt")])
        .await
        .unwrap();
    assert_eq!(e.head(), before);
    assert!(!e.path().join("first.txt").exists());
    assert!(e.git.operation_state(&e.info).await.unwrap().is_none());
}

#[tokio::test]
async fn a_commit_already_in_the_history_is_dropped_kept_or_stops_the_sequence_as_chosen() {
    let setup = |p: &Path| {
        write(p, "a.txt", "base\n");
        commit_all(p, "base");
        run_git(p, &["checkout", "-q", "-b", "side"]);
        write(p, "x.txt", "x\n");
        commit_all(p, "side x");
        run_git(p, &["checkout", "-q", "main"]);
        write(p, "x.txt", "x\n");
        commit_all(p, "main already has x");
    };
    let dropped = env_with(setup).await;
    let before = dropped.head();
    let outcome = dropped
        .run(SequenceKind::CherryPick, &["side"], None, EmptyPolicy::Drop)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        SequenceOutcome::Done {
            created: 0,
            head: before.clone()
        }
    );
    let kept = env_with(setup).await;
    let outcome = kept
        .run(SequenceKind::CherryPick, &["side"], None, EmptyPolicy::Keep)
        .await
        .unwrap();
    assert!(
        matches!(outcome, SequenceOutcome::Done { created: 1, .. }),
        "{outcome:?}"
    );
    assert_eq!(kept.out(&["show", "--stat", "--format=", "HEAD"]), "");
    let stopped = env_with(setup).await;
    let outcome = stopped
        .run(SequenceKind::CherryPick, &["side"], None, EmptyPolicy::Stop)
        .await
        .unwrap();
    assert!(
        matches!(outcome, SequenceOutcome::Stopped { .. }),
        "{outcome:?}"
    );
    assert!(
        stopped
            .git
            .operation_state(&stopped.info)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        stopped
            .git
            .list_conflicts(&stopped.info)
            .await
            .unwrap()
            .is_empty()
    );
    let skipped = stopped.git.skip_operation(&stopped.info).await.unwrap();
    assert!(skipped.finished);
    assert_eq!(stopped.head(), before_of(&stopped));
}

fn before_of(e: &Env) -> String {
    e.out(&["rev-parse", "main"])
}

fn merge_history(p: &Path) {
    write(p, "a.txt", "base\n");
    commit_all(p, "base");
    run_git(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "f.txt", "f\n");
    commit_all(p, "feature work");
    run_git(p, &["checkout", "-q", "main"]);
    write(p, "m.txt", "m\n");
    commit_all(p, "main work");
    run_git(
        p,
        &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"],
    );
}

#[tokio::test]
async fn a_merge_commit_needs_an_explicit_parent_and_cannot_be_mixed_with_others() {
    let e = env_with(|p| {
        merge_history(p);
    })
    .await;
    let merge = e.oid("main");
    let plain = e.oid("main~1");
    let preview = e
        .git
        .sequence_preview(&e.info, SequenceKind::Revert, std::slice::from_ref(&merge))
        .await
        .unwrap();
    assert_eq!(preview.merges, 1);
    assert_eq!(preview.commits[0].parents, 2);
    assert_eq!(
        e.run(SequenceKind::Revert, &["main"], None, EmptyPolicy::Stop)
            .await
            .unwrap_err(),
        SequenceError::MergeNeedsParent
    );
    assert_eq!(
        e.run(SequenceKind::Revert, &["main"], Some(3), EmptyPolicy::Stop)
            .await
            .unwrap_err(),
        SequenceError::BadMainline
    );
    let mixed = e
        .git
        .run_sequence(
            &e.info,
            &SequenceRequest {
                kind: SequenceKind::Revert,
                oids: &[merge.clone(), plain],
                mainline: Some(1),
                empty: EmptyPolicy::Stop,
                expected_head: &e.head(),
            },
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(mixed, SequenceError::MergesMustStandAlone);
    let outcome = e
        .run(SequenceKind::Revert, &["main"], Some(1), EmptyPolicy::Stop)
        .await
        .unwrap();
    assert!(
        matches!(outcome, SequenceOutcome::Done { created: 1, .. }),
        "{outcome:?}"
    );
    assert!(!e.path().join("f.txt").exists());
    assert!(e.path().join("m.txt").exists());
}

#[tokio::test]
async fn revert_creates_the_inverse_commit_and_leaves_history_in_place() {
    let e = env_with(|p| {
        write(p, "a.txt", "one\n");
        commit_all(p, "one");
        write(p, "a.txt", "two\n");
        commit_all(p, "two");
        write(p, "b.txt", "later\n");
        commit_all(p, "later");
    })
    .await;
    let before = e.head();
    let two = e.oid("HEAD~1");
    let outcome = e
        .run(SequenceKind::Revert, &["HEAD~1"], None, EmptyPolicy::Stop)
        .await
        .unwrap();
    assert!(
        matches!(outcome, SequenceOutcome::Done { created: 1, .. }),
        "{outcome:?}"
    );
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "one\n"
    );
    assert!(
        e.out(&["log", "-1", "--format=%s"])
            .starts_with("Revert \"two\"")
    );
    assert_eq!(e.oid("HEAD~1"), before);
    assert_eq!(e.out(&["cat-file", "-t", &two]), "commit");
}

#[tokio::test]
async fn a_conflicting_revert_keeps_the_operation_visible_and_can_be_abandoned() {
    let e = env_with(|p| {
        write(p, "a.txt", "one\n");
        commit_all(p, "one");
        write(p, "a.txt", "two\n");
        commit_all(p, "two");
        write(p, "a.txt", "three\n");
        commit_all(p, "three");
    })
    .await;
    let before = e.head();
    let outcome = e
        .run(SequenceKind::Revert, &["HEAD~1"], None, EmptyPolicy::Stop)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        SequenceOutcome::Conflicts {
            files: vec![PathBuf::from("a.txt")]
        }
    );
    let op = e.git.operation_state(&e.info).await.unwrap().unwrap();
    assert_eq!(op.kind, OperationKind::Revert);
    assert!(
        op.theirs_label.contains("annulation"),
        "{}",
        op.theirs_label
    );
    e.git
        .resolve_with_side(&e.info, Path::new("a.txt"), ConflictSide::Ours)
        .await
        .unwrap();
    e.git
        .abort_operation(&e.info, &[PathBuf::from("a.txt")])
        .await
        .unwrap();
    assert_eq!(e.head(), before);
    assert!(e.git.operation_state(&e.info).await.unwrap().is_none());
}

#[tokio::test]
async fn bad_requests_change_nothing() {
    let e = env_with(side_branch).await;
    let head = e.head();
    let go = |oids: Vec<String>, expected: String| {
        let (git, info) = (e.git.clone(), e.info.clone());
        async move {
            git.run_sequence(
                &info,
                &SequenceRequest {
                    kind: SequenceKind::CherryPick,
                    oids: &oids,
                    mainline: None,
                    empty: EmptyPolicy::Drop,
                    expected_head: &expected,
                },
                None,
            )
            .await
        }
    };
    assert_eq!(
        go(vec![], head.clone()).await.unwrap_err(),
        SequenceError::NothingSelected
    );
    for bad in ["HEAD", "--abort", "abc", &"0".repeat(40)] {
        assert!(
            matches!(
                go(vec![bad.to_owned()], head.clone()).await,
                Err(SequenceError::InvalidCommit(_))
            ),
            "{bad}"
        );
    }
    assert_eq!(
        go(vec![e.oid("side")], "0".repeat(40)).await.unwrap_err(),
        SequenceError::HeadMoved
    );
    assert_eq!(e.head(), head);
    assert!(e.out(&["status", "--porcelain"]).is_empty());
}

#[tokio::test]
async fn local_changes_that_would_be_overwritten_stop_the_sequence_untouched() {
    let e = env_with(|p| {
        write(p, "a.txt", "base\n");
        commit_all(p, "base");
        run_git(p, &["checkout", "-q", "-b", "side"]);
        write(p, "a.txt", "side\n");
        commit_all(p, "side edit");
        run_git(p, &["checkout", "-q", "main"]);
        write(p, "b.txt", "b\n");
        commit_all(p, "main b");
    })
    .await;
    write(e.path(), "a.txt", "my unsaved work\n");
    let head = e.head();
    let error = e
        .run(SequenceKind::CherryPick, &["side"], None, EmptyPolicy::Drop)
        .await
        .unwrap_err();
    assert!(matches!(error, SequenceError::LocalChanges(_)), "{error:?}");
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "my unsaved work\n"
    );
    assert_eq!(e.head(), head);
}

#[tokio::test]
async fn another_operation_in_progress_blocks_a_sequence_and_the_profile_identity_is_used() {
    let e = env_with(side_branch).await;
    let profile = Profile {
        id: "p".into(),
        label: "P".into(),
        user_name: "Grace Hopper".into(),
        user_email: "grace@example.org".into(),
        signing: SigningPreference::None,
    };
    let oids = vec![e.oid("side~2")];
    let head = e.head();
    e.git
        .run_sequence(
            &e.info,
            &SequenceRequest {
                kind: SequenceKind::CherryPick,
                oids: &oids,
                mainline: None,
                empty: EmptyPolicy::Drop,
                expected_head: &head,
            },
            Some(&profile),
        )
        .await
        .unwrap();
    assert_eq!(
        e.out(&["log", "-1", "--format=%cn <%ce>"]),
        "Grace Hopper <grace@example.org>"
    );
    write(e.path(), "a.txt", "x\n");
    commit_all(e.path(), "x");
    run_git(e.path(), &["checkout", "-q", "-b", "other", "HEAD~1"]);
    write(e.path(), "a.txt", "y\n");
    commit_all(e.path(), "y");
    run_git(e.path(), &["checkout", "-q", "main"]);
    assert!(!git_output(e.path(), &["merge", "other"]).status.success());
    assert_eq!(
        e.run(SequenceKind::CherryPick, &["side"], None, EmptyPolicy::Drop)
            .await
            .unwrap_err(),
        SequenceError::OperationInProgress
    );
}
