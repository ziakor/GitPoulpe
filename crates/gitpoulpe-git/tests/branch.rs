mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::{commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{BranchError, Cancel, DeleteMode, Git};

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
    write(repo.path(), "common.txt", "common\n");
    commit_all(repo.path(), "base");
    let info = git.open(repo.path(), &Cancel::new()).await.unwrap();
    Fixture { repo, git, info }
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
    fn branches(&self) -> BTreeSet<String> {
        self.out(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn commit_on(&self, branch: &str, file: &str, message: &str) -> String {
        run_git(self.path(), &["switch", "-q", branch]);
        write(self.path(), file, message);
        commit_all(self.path(), message);
        self.out(&["rev-parse", "HEAD"])
    }
}

#[tokio::test]
async fn the_list_matches_what_git_reports_for_each_branch() {
    let f = fixture().await;
    run_git(f.path(), &["branch", "feature"]);
    let tip = f.commit_on("feature", "f.txt", "feature work");
    run_git(f.path(), &["switch", "-q", "main"]);
    let work = tempfile::tempdir().unwrap();
    let tree = work.path().join("wt");
    run_git(
        f.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "elsewhere",
            tree.to_str().unwrap(),
        ],
    );
    run_git(f.path(), &["config", "branch.feature.remote", "."]);
    run_git(
        f.path(),
        &["config", "branch.feature.merge", "refs/heads/main"],
    );

    let list = f.git.list_branches(&f.info).await.unwrap();
    let names: BTreeSet<String> = list.iter().map(|b| b.name.clone()).collect();
    assert_eq!(names, f.branches());
    for branch in &list {
        assert_eq!(
            branch.oid,
            f.out(&["rev-parse", &format!("refs/heads/{}", branch.name)])
        );
    }
    let get = |n: &str| list.iter().find(|b| b.name == n).unwrap();
    assert!(get("main").current && !get("feature").current);
    assert_eq!(get("feature").oid, tip);
    assert_eq!(get("feature").subject, "feature work");
    assert_eq!(get("feature").upstream.as_deref(), Some("main"));
    assert_eq!(get("main").upstream, None);
    assert_eq!(
        get("elsewhere")
            .other_worktree
            .as_deref()
            .map(|p| p.canonicalize().unwrap()),
        Some(tree.canonicalize().unwrap())
    );
    assert_eq!(
        get("main").other_worktree,
        None,
        "the current worktree is not 'another'"
    );
}

#[tokio::test]
async fn creating_renaming_and_selecting_a_branch_updates_the_references() {
    let f = fixture().await;
    let base = f.out(&["rev-parse", "HEAD"]);
    f.git.create_branch(&f.info, "topic", None).await.unwrap();
    assert!(f.branches().contains("topic"));
    assert_eq!(f.out(&["rev-parse", "topic"]), base);
    assert_eq!(
        f.out(&["branch", "--show-current"]),
        "main",
        "creating does not switch"
    );

    let second = f.commit_on("main", "b.txt", "second");
    f.git
        .create_branch(&f.info, "at-second", Some(&second))
        .await
        .unwrap();
    assert_eq!(f.out(&["rev-parse", "at-second"]), second);

    let (before, after) = f.git.switch_branch(&f.info, "topic").await.unwrap();
    assert_eq!(f.out(&["branch", "--show-current"]), "topic");
    assert_eq!(before.as_deref(), Some(second.as_str()));
    assert_eq!(after, base);
    assert!(
        !f.path().join("b.txt").exists(),
        "the worktree follows the branch"
    );

    f.git
        .rename_branch(&f.info, "topic", "renamed/topic")
        .await
        .unwrap();
    assert_eq!(
        f.branches(),
        BTreeSet::from(["main", "at-second", "renamed/topic"].map(str::to_owned))
    );
    assert_eq!(
        f.out(&["branch", "--show-current"]),
        "renamed/topic",
        "HEAD follows the rename"
    );
    assert_eq!(f.out(&["rev-parse", "renamed/topic"]), base);

    let list = f.git.list_branches(&f.info).await.unwrap();
    assert!(
        list.iter()
            .find(|b| b.name == "renamed/topic")
            .unwrap()
            .current
    );
}

#[tokio::test]
async fn invalid_duplicate_and_unknown_inputs_are_refused_without_touching_references() {
    let f = fixture().await;
    let before = f.out(&["for-each-ref"]);
    for name in [
        "", "a b", "x..y", "-x", "--force", "@", "HEAD", "x.lock", "a~b", "a:b", "a\nb", "end/",
        ".dot", "a//b",
    ] {
        assert_eq!(
            f.git.create_branch(&f.info, name, None).await,
            Err(BranchError::InvalidName),
            "{name:?}"
        );
    }
    assert_eq!(
        f.git.create_branch(&f.info, "main", None).await,
        Err(BranchError::AlreadyExists)
    );
    let tree = f.out(&["rev-parse", "HEAD^{tree}"]);
    for start in [
        "zz",
        "--all",
        "abc",
        "0123456789abcdef0123456789abcdef01234567",
        &tree,
    ] {
        assert_eq!(
            f.git.create_branch(&f.info, "t", Some(start)).await,
            Err(BranchError::InvalidStart),
            "{start:?}"
        );
    }
    f.git.create_branch(&f.info, "other", None).await.unwrap();
    assert_eq!(
        f.git.rename_branch(&f.info, "other", "main").await,
        Err(BranchError::AlreadyExists)
    );
    assert_eq!(
        f.git.rename_branch(&f.info, "ghost", "x").await,
        Err(BranchError::NoSuchBranch)
    );
    assert_eq!(
        f.git.switch_branch(&f.info, "ghost").await.unwrap_err(),
        BranchError::NoSuchBranch
    );
    assert_eq!(
        f.git
            .switch_branch(&f.info, "origin/main")
            .await
            .unwrap_err(),
        BranchError::NoSuchBranch,
        "a remote-tracking name is never turned into a new branch"
    );
    let after = f.out(&["for-each-ref"]);
    assert_eq!(
        after.lines().count(),
        before.lines().count() + 1,
        "only `other` was created"
    );
}

#[tokio::test]
async fn switching_never_destroys_local_changes() {
    let f = fixture().await;
    run_git(f.path(), &["branch", "other"]);
    f.commit_on("other", "a.txt", "other edits a");
    f.commit_on("main", "keep.txt", "main work");
    let head = f.out(&["rev-parse", "HEAD"]);

    write(f.path(), "a.txt", "my uncommitted edit\n");
    let error = f.git.switch_branch(&f.info, "other").await.unwrap_err();
    let BranchError::Refused(message) = error else {
        panic!("a conflicting local change must be refused: {error:?}")
    };
    assert!(message.contains("would be overwritten"), "{message}");
    assert_eq!(
        std::fs::read_to_string(f.path().join("a.txt")).unwrap(),
        "my uncommitted edit\n"
    );
    assert_eq!(f.out(&["rev-parse", "HEAD"]), head);
    assert_eq!(f.out(&["branch", "--show-current"]), "main");

    run_git(f.path(), &["checkout", "--", "a.txt"]);
    write(f.path(), "unrelated.txt", "untracked and harmless\n");
    write(f.path(), "common.txt", "common edited locally\n");
    f.git.switch_branch(&f.info, "other").await.unwrap();
    assert_eq!(
        std::fs::read_to_string(f.path().join("unrelated.txt")).unwrap(),
        "untracked and harmless\n"
    );
    assert_eq!(
        std::fs::read_to_string(f.path().join("common.txt")).unwrap(),
        "common edited locally\n",
        "a compatible local change travels with the switch"
    );

    run_git(f.path(), &["checkout", "--", "."]);
    run_git(f.path(), &["switch", "-q", "main"]);
    run_git(f.path(), &["branch", "withfile"]);
    run_git(f.path(), &["switch", "-q", "withfile"]);
    write(f.path(), "clash.txt", "tracked on withfile\n");
    commit_all(f.path(), "clash");
    run_git(f.path(), &["switch", "-q", "main"]);
    write(f.path(), "clash.txt", "untracked data that must survive\n");
    let error = f.git.switch_branch(&f.info, "withfile").await.unwrap_err();
    assert!(matches!(error, BranchError::Refused(_)), "{error:?}");
    assert_eq!(
        std::fs::read_to_string(f.path().join("clash.txt")).unwrap(),
        "untracked data that must survive\n"
    );
}

#[tokio::test]
async fn a_branch_used_by_another_worktree_is_explained_and_left_alone() {
    let f = fixture().await;
    let work = tempfile::tempdir().unwrap();
    let tree = work.path().join("wt");
    run_git(
        f.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "busy",
            tree.to_str().unwrap(),
        ],
    );
    let expected = tree.canonicalize().unwrap();
    let used = |e: BranchError| match e {
        BranchError::UsedByWorktree(p) => p.canonicalize().unwrap(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        used(f.git.switch_branch(&f.info, "busy").await.unwrap_err()),
        expected
    );
    assert_eq!(
        used(
            f.git
                .delete_branch(&f.info, "busy", &DeleteMode::Merged)
                .await
                .unwrap_err()
        ),
        expected
    );
    let tip = f.out(&["rev-parse", "busy"]);
    assert_eq!(
        used(
            f.git
                .delete_branch(&f.info, "busy", &DeleteMode::Force { expected_tip: tip })
                .await
                .unwrap_err()
        ),
        expected
    );
    assert!(f.branches().contains("busy"));
}

#[tokio::test]
async fn deleting_an_unmerged_branch_lists_exactly_the_commits_that_would_be_lost() {
    let f = fixture().await;
    run_git(f.path(), &["branch", "wip"]);
    let first = f.commit_on("wip", "w1.txt", "wip one");
    let second = f.commit_on("wip", "w2.txt", "wip two");
    run_git(f.path(), &["branch", "shared", &first]);
    run_git(f.path(), &["switch", "-q", "main"]);

    assert_eq!(
        f.git
            .delete_branch(&f.info, "wip", &DeleteMode::Merged)
            .await,
        Err(BranchError::NotMerged)
    );
    assert!(
        f.branches().contains("wip"),
        "a refused deletion leaves the branch"
    );

    let loss = f.git.branch_loss(&f.info, "wip").await.unwrap();
    assert_eq!(loss.tip, second);
    let lost: Vec<&str> = loss.commits.iter().map(|c| c.oid.as_str()).collect();
    assert_eq!(
        lost,
        vec![second.as_str()],
        "`shared` still reaches the first commit"
    );
    assert_eq!(loss.commits[0].subject, "wip two");
    assert!(!loss.truncated);

    assert_eq!(
        f.git
            .delete_branch(
                &f.info,
                "wip",
                &DeleteMode::Force {
                    expected_tip: first.clone()
                }
            )
            .await,
        Err(BranchError::Changed),
        "a confirmation about another tip does not authorise the deletion"
    );
    assert!(f.branches().contains("wip"));

    let deleted = f
        .git
        .delete_branch(
            &f.info,
            "wip",
            &DeleteMode::Force {
                expected_tip: second.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(deleted, second);
    assert!(!f.branches().contains("wip"));
    run_git(f.path(), &["branch", "wip", &deleted]);
    assert_eq!(
        f.out(&["rev-parse", "wip"]),
        second,
        "the reported tip restores the branch"
    );
}

#[tokio::test]
async fn the_loss_list_is_what_only_this_branch_reaches_and_is_capped() {
    let f = fixture().await;
    let total = gitpoulpe_core::LOSS_LIST_LIMIT + 25;
    let parent = f.out(&["rev-parse", "HEAD"]);
    let mut stream = format!("reset refs/heads/long\nfrom {parent}\n");
    for n in 0..total {
        stream.push_str(&format!(
            "commit refs/heads/long\ncommitter t <t@example.com> {} +0000\ndata 4\nc{:02}\n",
            1_700_000_000 + n,
            n % 100
        ));
    }
    let mut child = std::process::Command::new("git")
        .current_dir(f.path())
        .args(["fast-import", "--quiet", "--force"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stream.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());

    let loss = f.git.branch_loss(&f.info, "long").await.unwrap();
    assert!(loss.truncated);
    assert_eq!(loss.commits.len(), gitpoulpe_core::LOSS_LIST_LIMIT);
    let all = f.out(&["rev-list", "main..long"]);
    assert_eq!(all.lines().count(), total);
    assert!(loss.commits.iter().all(|c| all.lines().any(|l| l == c.oid)));
}

#[tokio::test]
async fn the_current_branch_cannot_be_deleted_and_a_merged_one_can() {
    let f = fixture().await;
    assert_eq!(
        f.git
            .delete_branch(&f.info, "main", &DeleteMode::Merged)
            .await,
        Err(BranchError::IsCurrent)
    );
    f.git.create_branch(&f.info, "done", None).await.unwrap();
    let tip = f.out(&["rev-parse", "done"]);
    assert_eq!(
        f.git
            .delete_branch(&f.info, "done", &DeleteMode::Merged)
            .await,
        Ok(tip)
    );
    assert!(!f.branches().contains("done"));
    assert_eq!(
        f.git
            .delete_branch(&f.info, "done", &DeleteMode::Merged)
            .await,
        Err(BranchError::NoSuchBranch)
    );
}

#[tokio::test]
async fn a_repository_busy_with_another_git_process_is_reported_not_forced() {
    let f = fixture().await;
    run_git(f.path(), &["branch", "other"]);
    write(f.path(), ".git/index.lock", "");
    let error = f.git.switch_branch(&f.info, "other").await.unwrap_err();
    assert!(matches!(error, BranchError::IndexLocked(_)), "{error:?}");
    assert!(
        f.path().join(".git/index.lock").exists(),
        "the lock is never removed"
    );
}
