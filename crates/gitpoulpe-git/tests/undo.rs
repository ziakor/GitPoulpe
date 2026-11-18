mod common;

use std::path::Path;

use common::{commit_all, git as run_git, git_output, init_repo, write};
use gitpoulpe_core::{SessionInfo, UndoDirection, UndoKind, UndoRecord, UndoState};
use gitpoulpe_git::{Cancel, Git, UndoError};

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
    async fn go(&self, record: &mut UndoRecord, direction: UndoDirection) -> Result<(), UndoError> {
        self.git.undo_redo(&self.info, record, direction).await?;
        record.state = match direction {
            UndoDirection::Undo => UndoState::Undone,
            UndoDirection::Redo => UndoState::Done,
        };
        Ok(())
    }
}

async fn env() -> Env {
    let repo = init_repo();
    write(repo.path(), "a.txt", "one\n");
    commit_all(repo.path(), "one");
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null");
    let info = git.open(repo.path(), &Cancel::new()).await.unwrap();
    Env { repo, git, info }
}

fn record(kind: UndoKind) -> UndoRecord {
    UndoRecord {
        id: 1,
        at: 0,
        label: "test".into(),
        kind,
        state: UndoState::Done,
        limit: None,
    }
}

fn ref_move(reference: &str, before: &str, after: &str) -> UndoRecord {
    record(UndoKind::RefMove {
        reference: reference.into(),
        before: before.into(),
        after: after.into(),
    })
}

#[tokio::test]
async fn a_commit_is_undone_then_redone_and_unrelated_local_work_is_kept() {
    let e = env().await;
    let before = e.head();
    write(e.path(), "b.txt", "two\n");
    commit_all(e.path(), "two");
    let after = e.head();
    write(e.path(), "unrelated.txt", "mine\n");
    run_git(e.path(), &["add", "unrelated.txt"]);
    let mut r = ref_move("refs/heads/main", &before, &after);
    e.go(&mut r, UndoDirection::Undo).await.unwrap();
    assert_eq!(e.head(), before);
    assert!(!e.path().join("b.txt").exists());
    assert_eq!(
        std::fs::read_to_string(e.path().join("unrelated.txt")).unwrap(),
        "mine\n"
    );
    e.go(&mut r, UndoDirection::Redo).await.unwrap();
    assert_eq!(e.head(), after);
    assert!(e.path().join("b.txt").exists());
}

#[tokio::test]
async fn a_change_made_by_another_tool_prevents_a_silent_undo() {
    let e = env().await;
    let before = e.head();
    write(e.path(), "b.txt", "two\n");
    commit_all(e.path(), "two");
    let after = e.head();
    let mut r = ref_move("refs/heads/main", &before, &after);
    write(e.path(), "c.txt", "external\n");
    commit_all(e.path(), "made outside GitPoulpe");
    let external = e.head();
    let error = e.go(&mut r, UndoDirection::Undo).await.unwrap_err();
    assert!(matches!(error, UndoError::StateChanged { .. }), "{error:?}");
    assert_eq!(e.head(), external);
    assert!(e.path().join("c.txt").exists());
}

#[tokio::test]
async fn a_branch_that_is_not_checked_out_moves_only_if_it_is_where_the_record_says() {
    let e = env().await;
    let base = e.head();
    run_git(e.path(), &["checkout", "-q", "-b", "other"]);
    write(e.path(), "o.txt", "o\n");
    commit_all(e.path(), "other work");
    let after = e.head();
    run_git(e.path(), &["checkout", "-q", "main"]);
    let mut r = ref_move("refs/heads/other", &base, &after);
    e.go(&mut r, UndoDirection::Undo).await.unwrap();
    assert_eq!(e.out(&["rev-parse", "refs/heads/other"]), base);
    assert_eq!(e.head(), base);
    assert!(e.path().join("a.txt").exists());
    e.go(&mut r, UndoDirection::Redo).await.unwrap();
    assert_eq!(e.out(&["rev-parse", "refs/heads/other"]), after);
}

#[tokio::test]
async fn an_undo_that_would_overwrite_local_work_is_refused_and_changes_nothing() {
    let e = env().await;
    let before = e.head();
    write(e.path(), "a.txt", "two\n");
    commit_all(e.path(), "two");
    let after = e.head();
    write(e.path(), "a.txt", "my unsaved edit\n");
    let mut r = ref_move("refs/heads/main", &before, &after);
    let error = e.go(&mut r, UndoDirection::Undo).await.unwrap_err();
    assert!(
        matches!(error, UndoError::Exec(_) | UndoError::Refused(_)),
        "{error:?}"
    );
    assert_eq!(e.head(), after);
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "my unsaved edit\n"
    );
}

#[tokio::test]
async fn branches_and_tags_can_be_created_and_deleted_back_and_forth() {
    let e = env().await;
    let tip = e.head();
    run_git(e.path(), &["branch", "feature"]);
    let mut created = record(UndoKind::BranchCreate {
        name: "feature".into(),
        tip: tip.clone(),
    });
    e.go(&mut created, UndoDirection::Undo).await.unwrap();
    assert!(e.out(&["for-each-ref", "refs/heads/feature"]).is_empty());
    e.go(&mut created, UndoDirection::Redo).await.unwrap();
    assert_eq!(e.out(&["rev-parse", "feature"]), tip);
    let mut deleted = record(UndoKind::BranchDelete {
        name: "gone".into(),
        tip: tip.clone(),
    });
    deleted.state = UndoState::Done;
    e.go(&mut deleted, UndoDirection::Undo).await.unwrap();
    assert_eq!(e.out(&["rev-parse", "gone"]), tip);
    assert_eq!(
        e.go(&mut deleted, UndoDirection::Undo).await.unwrap_err(),
        UndoError::WrongState
    );
    e.go(&mut deleted, UndoDirection::Redo).await.unwrap();
    assert!(e.out(&["for-each-ref", "refs/heads/gone"]).is_empty());
    run_git(e.path(), &["tag", "-a", "-m", "note", "v1"]);
    let object = e.out(&["rev-parse", "refs/tags/v1"]);
    let mut tag = record(UndoKind::TagCreate {
        name: "v1".into(),
        object: object.clone(),
    });
    e.go(&mut tag, UndoDirection::Undo).await.unwrap();
    assert!(e.out(&["tag", "--list"]).is_empty());
    e.go(&mut tag, UndoDirection::Redo).await.unwrap();
    assert_eq!(e.out(&["rev-parse", "refs/tags/v1"]), object);
    assert_eq!(e.out(&["cat-file", "-t", "v1"]), "tag");
}

#[tokio::test]
async fn names_taken_in_the_meantime_and_the_current_branch_are_protected() {
    let e = env().await;
    let tip = e.head();
    run_git(e.path(), &["branch", "feature"]);
    let mut created = record(UndoKind::BranchCreate {
        name: "feature".into(),
        tip: tip.clone(),
    });
    e.go(&mut created, UndoDirection::Undo).await.unwrap();
    run_git(e.path(), &["branch", "feature"]);
    assert_eq!(
        e.go(&mut created, UndoDirection::Redo).await.unwrap_err(),
        UndoError::NameTaken
    );
    let mut current = record(UndoKind::BranchCreate {
        name: "main".into(),
        tip,
    });
    assert_eq!(
        e.go(&mut current, UndoDirection::Undo).await.unwrap_err(),
        UndoError::CheckedOut
    );
    assert_eq!(e.out(&["symbolic-ref", "--short", "HEAD"]), "main");
    write(e.path(), "x.txt", "x\n");
    commit_all(e.path(), "moves main");
    let mut stale = record(UndoKind::BranchCreate {
        name: "feature".into(),
        tip: e.out(&["rev-parse", "HEAD"]),
    });
    assert!(matches!(
        e.go(&mut stale, UndoDirection::Undo).await.unwrap_err(),
        UndoError::StateChanged { .. }
    ));
}

#[tokio::test]
async fn a_rename_is_undone_by_renaming_back_unless_the_old_name_is_taken() {
    let e = env().await;
    run_git(e.path(), &["branch", "newname"]);
    let mut r = record(UndoKind::BranchRename {
        old: "oldname".into(),
        new: "newname".into(),
    });
    e.go(&mut r, UndoDirection::Undo).await.unwrap();
    assert!(
        e.out(&["for-each-ref", "refs/heads/oldname"])
            .contains("oldname")
    );
    assert!(e.out(&["for-each-ref", "refs/heads/newname"]).is_empty());
    run_git(e.path(), &["branch", "newname"]);
    assert_eq!(
        e.go(&mut r, UndoDirection::Redo).await.unwrap_err(),
        UndoError::NameTaken
    );
}

#[tokio::test]
async fn operations_without_an_inverse_are_explained_and_a_missing_commit_is_reported() {
    let e = env().await;
    let mut push = record(UndoKind::Irreversible {
        why: "le distant a déjà reçu les commits".into(),
    });
    assert_eq!(
        e.go(&mut push, UndoDirection::Undo).await.unwrap_err(),
        UndoError::NotReversible("le distant a déjà reçu les commits".into())
    );
    let head = e.head();
    let mut ghost = ref_move("refs/heads/main", &"ab".repeat(20), &head);
    assert_eq!(
        e.go(&mut ghost, UndoDirection::Undo).await.unwrap_err(),
        UndoError::ObjectMissing
    );
    assert_eq!(e.head(), head);
}

#[tokio::test]
async fn nothing_is_undone_during_another_operation() {
    let e = env().await;
    let base = e.head();
    run_git(e.path(), &["checkout", "-q", "-b", "side"]);
    write(e.path(), "a.txt", "side\n");
    commit_all(e.path(), "side");
    run_git(e.path(), &["checkout", "-q", "main"]);
    write(e.path(), "a.txt", "main\n");
    commit_all(e.path(), "main");
    assert!(!git_output(e.path(), &["merge", "side"]).status.success());
    let mut r = ref_move("refs/heads/main", &base, &e.head());
    assert_eq!(
        e.go(&mut r, UndoDirection::Undo).await.unwrap_err(),
        UndoError::OperationInProgress
    );
}

#[tokio::test]
async fn a_chain_of_operations_goes_back_and_forth_to_the_same_states() {
    let e = env().await;
    let c1 = e.head();
    write(e.path(), "b.txt", "2\n");
    commit_all(e.path(), "c2");
    let c2 = e.head();
    write(e.path(), "c.txt", "3\n");
    commit_all(e.path(), "c3");
    let c3 = e.head();
    let mut second = ref_move("refs/heads/main", &c2, &c3);
    let mut first = ref_move("refs/heads/main", &c1, &c2);
    assert!(matches!(
        e.go(&mut first, UndoDirection::Undo).await.unwrap_err(),
        UndoError::StateChanged { .. }
    ));
    e.go(&mut second, UndoDirection::Undo).await.unwrap();
    e.go(&mut first, UndoDirection::Undo).await.unwrap();
    assert_eq!(e.head(), c1);
    e.go(&mut first, UndoDirection::Redo).await.unwrap();
    e.go(&mut second, UndoDirection::Redo).await.unwrap();
    assert_eq!(e.head(), c3);
    assert!(e.path().join("c.txt").exists() && e.path().join("b.txt").exists());
}
