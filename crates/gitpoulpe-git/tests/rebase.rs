mod common;

use std::path::{Path, PathBuf};

use common::{commit_all, git as run_git, git_output, init_repo, write};
use gitpoulpe_core::{
    OperationKind, PlanItem, RebaseAction, RebaseOutcome, RebasePlanInfo, SessionInfo,
};
use gitpoulpe_git::{Cancel, ConflictSide, Git, RebaseError, validate_plan};

struct Env {
    repo: tempfile::TempDir,
    git: Git,
    info: SessionInfo,
    base: String,
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
    fn subjects(&self) -> Vec<String> {
        self.out(&["log", "--format=%s", &format!("{}..HEAD", self.base)])
            .lines()
            .rev()
            .map(str::to_owned)
            .collect()
    }
    async fn plan(&self) -> RebasePlanInfo {
        self.git.rebase_plan(&self.info, &self.base).await.unwrap()
    }
    async fn items(&self, spec: &[(&str, RebaseAction, Option<&str>)]) -> Vec<PlanItem> {
        let plan = self.plan().await;
        spec.iter()
            .map(|(subject, action, message)| PlanItem {
                oid: plan
                    .commits
                    .iter()
                    .find(|c| c.subject == *subject)
                    .unwrap()
                    .oid
                    .clone(),
                action: *action,
                message: message.map(str::to_owned),
            })
            .collect()
    }
    async fn run(&self, items: &[PlanItem]) -> Result<RebaseOutcome, RebaseError> {
        self.git
            .start_rebase(&self.info, &self.base, items, &self.head(), false, None)
            .await
    }
}

async fn env_with(setup: impl FnOnce(&Path)) -> Env {
    let repo = init_repo();
    let p = repo.path();
    write(p, "base.txt", "base\n");
    commit_all(p, "base");
    let base = String::from_utf8_lossy(&run_git(p, &["rev-parse", "HEAD"]))
        .trim()
        .to_owned();
    setup(p);
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com");
    let info = git.open(p, &Cancel::new()).await.unwrap();
    Env {
        repo,
        git,
        info,
        base,
    }
}

fn five(p: &Path) {
    for n in ["a", "b", "c", "d", "e"] {
        write(p, &format!("{n}.txt"), &format!("{n}\n"));
        commit_all(p, &format!("add {n}"));
    }
}

use RebaseAction::{Drop, Edit, Fixup, Pick, Reword, Squash};

#[tokio::test]
async fn the_plan_shows_the_base_and_the_commits_oldest_first() {
    let e = env_with(five).await;
    let plan = e.plan().await;
    assert_eq!(plan.base, e.base);
    assert_eq!(plan.head, e.head());
    assert_eq!(plan.branch.as_deref(), Some("main"));
    assert_eq!(
        plan.commits
            .iter()
            .map(|c| c.subject.as_str())
            .collect::<Vec<_>>(),
        ["add a", "add b", "add c", "add d", "add e"]
    );
}

#[tokio::test]
async fn reordering_independent_commits_changes_the_order_and_keeps_the_final_tree() {
    let e = env_with(five).await;
    let tree = e.out(&["rev-parse", "HEAD^{tree}"]);
    let items = e
        .items(&[
            ("add c", Pick, None),
            ("add a", Pick, None),
            ("add b", Pick, None),
            ("add e", Pick, None),
            ("add d", Pick, None),
        ])
        .await;
    let outcome = e.run(&items).await.unwrap();
    assert!(matches!(outcome, RebaseOutcome::Done { .. }), "{outcome:?}");
    assert_eq!(e.subjects(), ["add c", "add a", "add b", "add e", "add d"]);
    assert_eq!(e.out(&["rev-parse", "HEAD^{tree}"]), tree);
    assert!(e.git.operation_state(&e.info).await.unwrap().is_none());
}

#[tokio::test]
async fn drop_squash_fixup_and_reword_produce_exactly_the_announced_history() {
    let e = env_with(five).await;
    let items = e
        .items(&[
            ("add a", Pick, None),
            ("add b", Squash, None),
            ("add c", Fixup, None),
            ("add d", Drop, None),
            ("add e", Reword, Some("fifth, reworded\n\nwith a body")),
        ])
        .await;
    let outcome = e.run(&items).await.unwrap();
    assert!(matches!(outcome, RebaseOutcome::Done { .. }), "{outcome:?}");
    let subjects = e.subjects();
    assert_eq!(subjects.len(), 2, "{subjects:?}");
    assert_eq!(subjects[0], "add a");
    assert_eq!(subjects[1], "fifth, reworded");
    assert_eq!(e.out(&["log", "-1", "--format=%b"]), "with a body");
    assert!(
        e.out(&["show", "--stat", "--format=", "HEAD~1"])
            .contains("b.txt")
    );
    assert!(
        e.out(&["show", "--stat", "--format=", "HEAD~1"])
            .contains("c.txt")
    );
    assert!(!e.path().join("d.txt").exists());
    assert!(e.path().join("e.txt").exists());
}

#[tokio::test]
async fn incompatible_plans_are_detected_before_anything_is_rewritten() {
    let e = env_with(five).await;
    let plan = e.plan().await;
    let head = e.head();
    let all = |actions: &[RebaseAction]| -> Vec<PlanItem> {
        plan.commits
            .iter()
            .zip(actions)
            .map(|(c, a)| PlanItem {
                oid: c.oid.clone(),
                action: *a,
                message: None,
            })
            .collect()
    };
    assert_eq!(
        validate_plan(&all(&[Squash, Pick, Pick, Pick, Pick]), &plan.commits),
        Err(RebaseError::FirstCannotMeld)
    );
    assert_eq!(
        validate_plan(&all(&[Drop, Fixup, Pick, Pick, Pick]), &plan.commits),
        Err(RebaseError::FirstCannotMeld)
    );
    assert_eq!(
        validate_plan(&all(&[Drop; 5]), &plan.commits),
        Err(RebaseError::NothingKept)
    );
    assert!(matches!(
        validate_plan(&all(&[Reword, Pick, Pick, Pick, Pick]), &plan.commits),
        Err(RebaseError::EmptyMessage(_))
    ));
    let missing = all(&[Pick, Pick, Pick, Pick, Pick])[..4].to_vec();
    assert_eq!(
        validate_plan(&missing, &plan.commits),
        Err(RebaseError::PlanMismatch)
    );
    let mut twice = all(&[Pick, Pick, Pick, Pick, Pick]);
    twice[4] = twice[0].clone();
    assert_eq!(
        validate_plan(&twice, &plan.commits),
        Err(RebaseError::PlanMismatch)
    );
    assert_eq!(
        e.run(&all(&[Squash, Pick, Pick, Pick, Pick]))
            .await
            .unwrap_err(),
        RebaseError::FirstCannotMeld
    );
    assert_eq!(e.head(), head);
    assert!(e.out(&["for-each-ref", "refs/gitpoulpe"]).is_empty());
    assert!(e.git.operation_state(&e.info).await.unwrap().is_none());
}

#[tokio::test]
async fn bad_bases_and_merges_in_the_range_are_refused() {
    let e = env_with(|p| {
        write(p, "a.txt", "a\n");
        commit_all(p, "add a");
        run_git(p, &["checkout", "-q", "-b", "side"]);
        write(p, "s.txt", "s\n");
        commit_all(p, "side");
        run_git(p, &["checkout", "-q", "main"]);
        write(p, "m.txt", "m\n");
        commit_all(p, "main");
        run_git(p, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    })
    .await;
    assert_eq!(
        e.git.rebase_plan(&e.info, &e.base).await.unwrap_err(),
        RebaseError::HasMerges
    );
    let side = e.out(&["rev-parse", "side"]);
    assert_eq!(
        e.git.rebase_plan(&e.info, &e.head()).await.unwrap_err(),
        RebaseError::NothingToRebase
    );
    let unrelated = {
        run_git(e.path(), &["checkout", "-q", "--orphan", "other"]);
        write(e.path(), "o.txt", "o\n");
        commit_all(e.path(), "orphan");
        let oid = e.out(&["rev-parse", "HEAD"]);
        run_git(e.path(), &["checkout", "-q", "main"]);
        oid
    };
    assert_eq!(
        e.git.rebase_plan(&e.info, &unrelated).await.unwrap_err(),
        RebaseError::BaseNotAncestor
    );
    for bad in ["HEAD", "main", "--abort", "abc"] {
        assert_eq!(
            e.git.rebase_plan(&e.info, bad).await.unwrap_err(),
            RebaseError::InvalidBase,
            "{bad}"
        );
    }
    let _ = side;
}

#[tokio::test]
async fn a_conflict_names_the_sides_and_a_resolved_rebase_continues_to_the_end() {
    let e = env_with(|p| {
        write(p, "f.txt", "one\n");
        commit_all(p, "first");
        write(p, "f.txt", "two\n");
        commit_all(p, "second");
        write(p, "g.txt", "g\n");
        commit_all(p, "third");
    })
    .await;
    let items = e
        .items(&[
            ("third", Pick, None),
            ("second", Pick, None),
            ("first", Pick, None),
        ])
        .await;
    let outcome = e.run(&items).await.unwrap();
    let RebaseOutcome::Conflicts { files, .. } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(files, vec![PathBuf::from("f.txt")]);
    let op = e.git.operation_state(&e.info).await.unwrap().unwrap();
    assert_eq!(op.kind, OperationKind::Rebase);
    assert!(
        op.theirs_label.contains("votre commit rejoué"),
        "{}",
        op.theirs_label
    );
    e.git
        .resolve_with_side(&e.info, Path::new("f.txt"), ConflictSide::Theirs)
        .await
        .unwrap();
    let mut remembered = vec![PathBuf::from("f.txt")];
    loop {
        let step = e
            .git
            .continue_operation(&e.info, None, &remembered)
            .await
            .unwrap();
        if step.finished {
            break;
        }
        let more = e.git.list_conflicts(&e.info).await.unwrap();
        if let Some(file) = more.first() {
            e.git
                .resolve_with_side(&e.info, &file.path, ConflictSide::Theirs)
                .await
                .unwrap();
            remembered = vec![file.path.clone()];
        }
    }
    assert!(e.git.operation_state(&e.info).await.unwrap().is_none());
    assert_eq!(e.subjects().len(), 3);
}

#[tokio::test]
async fn abandoning_restores_the_old_tip_which_stays_identifiable() {
    let e = env_with(|p| {
        write(p, "f.txt", "one\n");
        commit_all(p, "first");
        write(p, "f.txt", "two\n");
        commit_all(p, "second");
    })
    .await;
    let old = e.head();
    let items = e
        .items(&[("second", Pick, None), ("first", Pick, None)])
        .await;
    let outcome = e.run(&items).await.unwrap();
    let RebaseOutcome::Conflicts { marker, .. } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(e.out(&["rev-parse", &marker]), old);
    e.git
        .abort_operation(&e.info, &[PathBuf::from("f.txt")])
        .await
        .unwrap();
    assert_eq!(e.head(), old);
    assert!(e.git.operation_state(&e.info).await.unwrap().is_none());
    assert_eq!(e.out(&["rev-parse", &marker]), old);
    assert_eq!(e.out(&["rev-parse", "ORIG_HEAD"]), old);
}

#[tokio::test]
async fn an_edit_step_pauses_the_rebase_and_continuing_finishes_it() {
    let e = env_with(five).await;
    let items = e
        .items(&[
            ("add a", Pick, None),
            ("add b", Edit, None),
            ("add c", Pick, None),
            ("add d", Pick, None),
            ("add e", Pick, None),
        ])
        .await;
    let outcome = e.run(&items).await.unwrap();
    assert!(
        matches!(outcome, RebaseOutcome::Paused { .. }),
        "{outcome:?}"
    );
    assert_eq!(e.out(&["log", "-1", "--format=%s"]), "add b");
    assert!(!e.path().join("c.txt").exists());
    assert!(e.git.list_conflicts(&e.info).await.unwrap().is_empty());
    run_git(
        e.path(),
        &[
            "commit",
            "--amend",
            "-q",
            "-m",
            "add b, amended during the pause",
        ],
    );
    let done = e.git.continue_operation(&e.info, None, &[]).await.unwrap();
    assert!(done.finished, "{}", done.message);
    assert!(
        e.subjects()
            .contains(&"add b, amended during the pause".to_owned())
    );
    assert!(e.path().join("e.txt").exists());
}

#[tokio::test]
async fn stale_plans_dirty_trees_and_other_operations_stop_before_any_rewrite() {
    let e = env_with(five).await;
    let items = e
        .items(&[
            ("add a", Pick, None),
            ("add b", Pick, None),
            ("add c", Pick, None),
            ("add d", Pick, None),
            ("add e", Pick, None),
        ])
        .await;
    let head = e.head();
    assert_eq!(
        e.git
            .start_rebase(&e.info, &e.base, &items, &"0".repeat(40), false, None)
            .await
            .unwrap_err(),
        RebaseError::HeadMoved
    );
    write(e.path(), "a.txt", "dirty\n");
    assert_eq!(e.run(&items).await.unwrap_err(), RebaseError::LocalChanges);
    assert_eq!(e.head(), head);
    assert!(e.out(&["for-each-ref", "refs/gitpoulpe"]).is_empty());
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "dirty\n"
    );
    run_git(e.path(), &["checkout", "-q", "--", "a.txt"]);
    run_git(e.path(), &["checkout", "-q", "-b", "side", "HEAD~1"]);
    write(e.path(), "e.txt", "conflict\n");
    commit_all(e.path(), "side e");
    run_git(e.path(), &["checkout", "-q", "main"]);
    assert!(!git_output(e.path(), &["merge", "side"]).status.success());
    assert_eq!(
        e.run(&items).await.unwrap_err(),
        RebaseError::OperationInProgress
    );
}

#[tokio::test]
async fn autostash_lets_a_dirty_tree_be_rebased_and_gets_the_work_back() {
    let e = env_with(five).await;
    let items = e
        .items(&[
            ("add b", Pick, None),
            ("add a", Pick, None),
            ("add c", Pick, None),
            ("add d", Pick, None),
            ("add e", Pick, None),
        ])
        .await;
    write(e.path(), "a.txt", "work in progress\n");
    let head = e.head();
    let outcome = e
        .git
        .start_rebase(&e.info, &e.base, &items, &head, true, None)
        .await
        .unwrap();
    assert!(matches!(outcome, RebaseOutcome::Done { .. }), "{outcome:?}");
    assert_eq!(
        std::fs::read_to_string(e.path().join("a.txt")).unwrap(),
        "work in progress\n"
    );
    assert!(e.out(&["stash", "list"]).is_empty());
    assert_eq!(e.subjects()[0], "add b");
}

fn zero_one_two(p: &Path) {
    write(p, "f.txt", "one\n");
    commit_all(p, "first");
    write(p, "f.txt", "two\n");
    commit_all(p, "second");
}

async fn walk_to_the_end(e: &Env) {
    let mut remembered: Vec<PathBuf> = Vec::new();
    for _ in 0..6 {
        let files = e.git.list_conflicts(&e.info).await.unwrap();
        for file in &files {
            e.git
                .resolve_with_side(&e.info, &file.path, ConflictSide::Theirs)
                .await
                .unwrap();
            remembered.push(file.path.clone());
        }
        let step = e
            .git
            .continue_operation(&e.info, None, &remembered)
            .await
            .unwrap();
        if step.finished {
            return;
        }
    }
    panic!("the rebase never finished");
}

#[tokio::test]
async fn a_reword_after_a_pause_still_finds_its_message_when_the_rebase_resumes() {
    let e = env_with(zero_one_two).await;
    let items = e
        .items(&[
            ("second", Pick, None),
            ("first", Reword, Some("first, reworded after a conflict")),
        ])
        .await;
    let outcome = e.run(&items).await.unwrap();
    assert!(
        matches!(outcome, RebaseOutcome::Conflicts { .. }),
        "{outcome:?}"
    );
    assert!(
        e.path().join(".git/gitpoulpe-rebase").is_dir(),
        "the files must outlive the first call"
    );
    walk_to_the_end(&e).await;
    assert_eq!(
        e.out(&["log", "-1", "--format=%s"]),
        "first, reworded after a conflict"
    );
    assert!(
        !e.path().join(".git/gitpoulpe-rebase").exists(),
        "cleaned up when the rebase ends"
    );
}

#[tokio::test]
async fn the_rebase_files_are_private_and_removed_on_abort_and_on_refusal() {
    use std::os::unix::fs::PermissionsExt;
    let e = env_with(zero_one_two).await;
    let items = e
        .items(&[("second", Pick, None), ("first", Reword, Some("later"))])
        .await;
    e.run(&items).await.unwrap();
    let dir = e.path().join(".git/gitpoulpe-rebase");
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    e.git
        .abort_operation(&e.info, &[PathBuf::from("f.txt")])
        .await
        .unwrap();
    assert!(!dir.exists());
    write(e.path(), "f.txt", "dirty\n");
    let plan = e.plan().await;
    let kept: Vec<PlanItem> = plan
        .commits
        .iter()
        .map(|c| PlanItem {
            oid: c.oid.clone(),
            action: Reword,
            message: Some("x".into()),
        })
        .collect();
    assert_eq!(e.run(&kept).await.unwrap_err(), RebaseError::LocalChanges);
    assert!(!dir.exists());
}

#[tokio::test]
async fn a_repository_path_with_quotes_and_spaces_cannot_break_or_inject_into_the_todo() {
    let parent = tempfile::tempdir().unwrap();
    let tricky = parent.path().join("it's a \"path\" $(touch pwned) here");
    std::fs::create_dir(&tricky).unwrap();
    run_git(&tricky, &["init", "-q", "-b", "main"]);
    write(&tricky, "base.txt", "base\n");
    commit_all(&tricky, "base");
    let base = String::from_utf8_lossy(&run_git(&tricky, &["rev-parse", "HEAD"]))
        .trim()
        .to_owned();
    write(&tricky, "a.txt", "a\n");
    commit_all(&tricky, "add a");
    write(&tricky, "b.txt", "b\n");
    commit_all(&tricky, "add b");
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null")
        .with_env("GIT_AUTHOR_NAME", "t")
        .with_env("GIT_AUTHOR_EMAIL", "t@example.com")
        .with_env("GIT_COMMITTER_NAME", "t")
        .with_env("GIT_COMMITTER_EMAIL", "t@example.com");
    let info = git.open(&tricky, &Cancel::new()).await.unwrap();
    let plan = git.rebase_plan(&info, &base).await.unwrap();
    let items: Vec<PlanItem> = plan
        .commits
        .iter()
        .map(|c| PlanItem {
            oid: c.oid.clone(),
            action: Reword,
            message: Some(format!("{} (reworded)", c.subject)),
        })
        .collect();
    let head = String::from_utf8_lossy(&run_git(&tricky, &["rev-parse", "HEAD"]))
        .trim()
        .to_owned();
    let outcome = git
        .start_rebase(&info, &base, &items, &head, false, None)
        .await
        .unwrap();
    assert!(matches!(outcome, RebaseOutcome::Done { .. }), "{outcome:?}");
    assert_eq!(
        String::from_utf8_lossy(&run_git(&tricky, &["log", "-1", "--format=%s"])).trim(),
        "add b (reworded)"
    );
    assert!(!tricky.join("pwned").exists());
    assert!(!parent.path().join("pwned").exists());
}
