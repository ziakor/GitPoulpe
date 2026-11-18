mod common;

use std::path::Path;

use common::{commit_all, git as run_git, git_output, init_repo, write};
use gitpoulpe_core::{
    FlowKind, FlowOutcome, FlowStep, GitflowConfig, Operation, OperationKind, SessionInfo,
};
use gitpoulpe_git::{Cancel, Git, GitflowError};

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
    fn branches(&self) -> Vec<String> {
        self.out(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn parents(&self, rev: &str) -> usize {
        self.out(&["rev-list", "--parents", "-n", "1", rev])
            .split_whitespace()
            .count()
            - 1
    }
}

async fn env_with(setup: impl FnOnce(&Path)) -> Env {
    let repo = init_repo();
    write(repo.path(), "a.txt", "base\n");
    commit_all(repo.path(), "base");
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

async fn flow_env() -> Env {
    env_with(|p| {
        run_git(p, &["branch", "develop"]);
    })
    .await
}

async fn start(env: &Env, kind: FlowKind, name: &str) {
    let plan = env
        .git
        .gitflow_plan(&env.info, kind, false, name)
        .await
        .unwrap();
    env.git
        .gitflow_run(&env.info, &plan, false, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn effective_conventions_are_visible_and_reading_or_planning_modifies_nothing() {
    let env = env_with(|p| {
        run_git(p, &["branch", "-m", "main", "master"]);
        run_git(p, &["branch", "develop"]);
    })
    .await;
    let before = env.out(&["config", "--local", "--list"]);
    let view = env.git.gitflow_view(&env.info).await.unwrap();
    assert!(!view.saved);
    assert_eq!(view.config.main, "master");
    assert_eq!(view.config.develop, "develop");
    assert_eq!(view.config.feature, "feature/");
    assert!(view.main_exists && view.develop_exists);
    env.git
        .gitflow_plan(&env.info, FlowKind::Feature, false, "login")
        .await
        .unwrap();
    assert_eq!(env.out(&["config", "--local", "--list"]), before);
    assert_eq!(env.branches(), ["develop", "master"]);
}

#[tokio::test]
async fn saving_validates_names_asks_before_creating_develop_and_then_is_reported_as_saved() {
    let env = env_with(|_| {}).await;
    let config = GitflowConfig::default();
    assert_eq!(
        env.git
            .gitflow_save(&env.info, &config, false)
            .await
            .unwrap_err(),
        GitflowError::DevelopMissing("develop".to_owned())
    );
    assert_eq!(env.branches(), ["main"]);
    for bad in [
        GitflowConfig {
            feature: "feature".into(),
            ..config.clone()
        },
        GitflowConfig {
            develop: "main".into(),
            ..config.clone()
        },
        GitflowConfig {
            release: "feature/rel/".into(),
            ..config.clone()
        },
        GitflowConfig {
            hotfix: "-x/".into(),
            ..config.clone()
        },
        GitflowConfig {
            develop: "feature/dev".into(),
            ..config.clone()
        },
    ] {
        let error = env
            .git
            .gitflow_save(&env.info, &bad, true)
            .await
            .unwrap_err();
        assert!(matches!(error, GitflowError::InvalidConfig(_)), "{error:?}");
    }
    assert_eq!(env.branches(), ["main"]);
    assert!(
        env.out(&["config", "--local", "--list"])
            .lines()
            .all(|l| !l.starts_with("gitflow."))
    );
    let custom = GitflowConfig {
        feature: "feat/".into(),
        tag_prefix: "v".into(),
        ..config
    };
    let effects = env
        .git
        .gitflow_save(&env.info, &custom, true)
        .await
        .unwrap();
    assert!(effects.iter().any(|e| e.contains("develop")));
    assert_eq!(env.branches(), ["develop", "main"]);
    let view = env.git.gitflow_view(&env.info).await.unwrap();
    assert!(view.saved);
    assert_eq!(view.config, custom);
}

#[tokio::test]
async fn starting_creates_the_branch_from_the_right_base_and_announces_it_first() {
    let env = flow_env().await;
    run_git(env.path(), &["checkout", "-q", "develop"]);
    write(env.path(), "d.txt", "dev\n");
    commit_all(env.path(), "dev work");
    let develop = env.out(&["rev-parse", "develop"]);
    let main = env.out(&["rev-parse", "main"]);
    let plan = env
        .git
        .gitflow_plan(&env.info, FlowKind::Feature, false, "login")
        .await
        .unwrap();
    assert_eq!(
        plan.steps,
        [
            FlowStep::CreateBranch {
                name: "feature/login".into(),
                from: "develop".into()
            },
            FlowStep::Switch {
                branch: "feature/login".into()
            }
        ]
    );
    assert_eq!(env.branches(), ["develop", "main"]);
    start(&env, FlowKind::Feature, "login").await;
    assert_eq!(env.out(&["rev-parse", "HEAD"]), develop);
    assert_eq!(
        env.out(&["symbolic-ref", "--short", "HEAD"]),
        "feature/login"
    );
    start(&env, FlowKind::Hotfix, "1.0.1").await;
    assert_eq!(env.out(&["rev-parse", "HEAD"]), main);
    assert_eq!(
        env.out(&["symbolic-ref", "--short", "HEAD"]),
        "hotfix/1.0.1"
    );
    assert_eq!(
        env.git
            .gitflow_plan(&env.info, FlowKind::Feature, false, "login")
            .await
            .unwrap_err(),
        GitflowError::BranchExists("feature/login".into())
    );
    assert_eq!(
        env.git
            .gitflow_plan(&env.info, FlowKind::Feature, false, "bad name~")
            .await
            .unwrap_err(),
        GitflowError::InvalidName
    );
}

#[tokio::test]
async fn finishing_a_feature_merges_it_with_a_merge_commit_and_removes_the_branch() {
    let env = flow_env().await;
    start(&env, FlowKind::Feature, "login").await;
    write(env.path(), "f.txt", "feature\n");
    commit_all(env.path(), "feature work");
    run_git(env.path(), &["checkout", "-q", "develop"]);
    write(env.path(), "d.txt", "dev\n");
    commit_all(env.path(), "dev work");
    run_git(env.path(), &["checkout", "-q", "main"]);
    let plan = env
        .git
        .gitflow_plan(&env.info, FlowKind::Feature, true, "login")
        .await
        .unwrap();
    let described: Vec<String> = plan.steps.iter().map(FlowStep::describe).collect();
    assert_eq!(described.len(), 3, "{described:?}");
    assert!(described[1].contains("fusionner feature/login dans develop"));
    assert_eq!(env.branches().len(), 3);
    let outcome = env
        .git
        .gitflow_run(&env.info, &plan, true, None)
        .await
        .unwrap();
    assert!(matches!(outcome, FlowOutcome::Finished { .. }));
    assert_eq!(env.branches(), ["develop", "main"]);
    assert_eq!(env.parents("develop"), 2);
    assert_eq!(env.out(&["symbolic-ref", "--short", "HEAD"]), "develop");
    assert_eq!(env.out(&["show", "develop:f.txt"]), "feature");
}

#[tokio::test]
async fn finishing_a_release_merges_into_main_tags_it_merges_into_develop_and_deletes_it() {
    let env = flow_env().await;
    start(&env, FlowKind::Release, "1.2.0").await;
    write(env.path(), "r.txt", "release fix\n");
    commit_all(env.path(), "release fix");
    run_git(env.path(), &["checkout", "-q", "main"]);
    write(env.path(), "m.txt", "main work\n");
    commit_all(env.path(), "main work");
    run_git(env.path(), &["checkout", "-q", "develop"]);
    write(env.path(), "d.txt", "dev\n");
    commit_all(env.path(), "dev work");
    let plan = env
        .git
        .gitflow_plan(&env.info, FlowKind::Release, true, "1.2.0")
        .await
        .unwrap();
    let described: Vec<String> = plan.steps.iter().map(FlowStep::describe).collect();
    assert!(
        described
            .iter()
            .any(|d| d.contains("tag annoté 1.2.0 sur main")),
        "{described:?}"
    );
    let outcome = env
        .git
        .gitflow_run(&env.info, &plan, true, None)
        .await
        .unwrap();
    assert!(matches!(outcome, FlowOutcome::Finished { .. }));
    assert_eq!(env.branches(), ["develop", "main"]);
    assert_eq!(env.out(&["cat-file", "-t", "refs/tags/1.2.0"]), "tag");
    assert_eq!(
        env.out(&["rev-parse", "refs/tags/1.2.0^{commit}"]),
        env.out(&["rev-parse", "main"])
    );
    assert_eq!(env.parents("main"), 2);
    assert_eq!(env.parents("develop"), 2);
    assert_eq!(env.out(&["show", "main:r.txt"]), "release fix");
    assert_eq!(env.out(&["show", "develop:r.txt"]), "release fix");
    assert_eq!(env.out(&["show", "develop:d.txt"]), "dev");
}

#[tokio::test]
async fn a_conflict_stops_the_flow_cleanly_and_finishing_again_resumes_after_the_resolution() {
    let env = flow_env().await;
    start(&env, FlowKind::Hotfix, "1.0.1").await;
    write(env.path(), "a.txt", "hotfix\n");
    commit_all(env.path(), "hotfix");
    run_git(env.path(), &["checkout", "-q", "develop"]);
    write(env.path(), "a.txt", "develop\n");
    commit_all(env.path(), "develop edit");
    run_git(env.path(), &["checkout", "-q", "main"]);
    let plan = env
        .git
        .gitflow_plan(&env.info, FlowKind::Hotfix, true, "1.0.1")
        .await
        .unwrap();
    let outcome = env
        .git
        .gitflow_run(&env.info, &plan, true, None)
        .await
        .unwrap();
    let FlowOutcome::Interrupted {
        done,
        at,
        remaining,
        files,
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    assert!(done.iter().any(|s| matches!(s, FlowStep::Tag { .. })));
    assert_eq!(
        at,
        FlowStep::Merge {
            source: "hotfix/1.0.1".into(),
            target: "develop".into()
        }
    );
    assert!(matches!(
        remaining.as_slice(),
        [FlowStep::DeleteBranch { .. }]
    ));
    assert_eq!(files, [Path::new("a.txt")]);
    let operation: Option<Operation> = env.git.operation_state(&env.info).await.unwrap();
    assert_eq!(operation.unwrap().kind, OperationKind::Merge);
    assert!(env.branches().contains(&"hotfix/1.0.1".to_owned()));
    assert_eq!(
        env.git
            .gitflow_run(&env.info, &plan, true, None)
            .await
            .unwrap_err(),
        GitflowError::OperationInProgress
    );
    write(env.path(), "a.txt", "resolved\n");
    run_git(env.path(), &["add", "a.txt"]);
    run_git(env.path(), &["commit", "-q", "--no-edit"]);
    let resumed = env
        .git
        .gitflow_plan(&env.info, FlowKind::Hotfix, true, "1.0.1")
        .await
        .unwrap();
    assert_eq!(
        resumed.steps,
        [FlowStep::DeleteBranch {
            name: "hotfix/1.0.1".into()
        }]
    );
    assert_eq!(resumed.already_done.len(), 3);
    let outcome = env
        .git
        .gitflow_run(&env.info, &resumed, true, None)
        .await
        .unwrap();
    assert!(matches!(outcome, FlowOutcome::Finished { .. }));
    assert_eq!(env.branches(), ["develop", "main"]);
    assert_eq!(env.out(&["show", "develop:a.txt"]), "resolved");
}

#[tokio::test]
async fn a_plan_that_went_stale_is_refused_and_nothing_is_executed() {
    let env = flow_env().await;
    start(&env, FlowKind::Feature, "login").await;
    write(env.path(), "f.txt", "feature\n");
    commit_all(env.path(), "feature work");
    let plan = env
        .git
        .gitflow_plan(&env.info, FlowKind::Feature, true, "login")
        .await
        .unwrap();
    run_git(env.path(), &["checkout", "-q", "develop"]);
    run_git(
        env.path(),
        &["merge", "-q", "--no-ff", "-m", "manual", "feature/login"],
    );
    let develop = env.out(&["rev-parse", "develop"]);
    assert_eq!(
        env.git
            .gitflow_run(&env.info, &plan, true, None)
            .await
            .unwrap_err(),
        GitflowError::PlanChanged
    );
    assert_eq!(env.out(&["rev-parse", "develop"]), develop);
    assert!(env.branches().contains(&"feature/login".to_owned()));
}

#[tokio::test]
async fn local_changes_block_the_flow_before_any_step_runs() {
    let env = flow_env().await;
    start(&env, FlowKind::Feature, "login").await;
    write(env.path(), "f.txt", "feature\n");
    commit_all(env.path(), "feature work");
    write(env.path(), "a.txt", "uncommitted\n");
    let plan = env
        .git
        .gitflow_plan(&env.info, FlowKind::Feature, true, "login")
        .await
        .unwrap();
    let error = env
        .git
        .gitflow_run(&env.info, &plan, true, None)
        .await
        .unwrap_err();
    assert!(matches!(error, GitflowError::LocalChanges(_)), "{error:?}");
    assert_eq!(
        env.out(&["symbolic-ref", "--short", "HEAD"]),
        "feature/login"
    );
    assert_eq!(env.out(&["show", "feature/login:a.txt"]), "base");
    assert_eq!(env.branches().len(), 3);
}

#[tokio::test]
async fn an_existing_tag_on_another_commit_is_never_moved_and_blocks_the_plan() {
    let env = flow_env().await;
    start(&env, FlowKind::Release, "2.0.0").await;
    write(env.path(), "r.txt", "r\n");
    commit_all(env.path(), "release work");
    run_git(env.path(), &["tag", "2.0.0", "main"]);
    let before = env.out(&["rev-parse", "refs/tags/2.0.0"]);
    assert_eq!(
        env.git
            .gitflow_plan(&env.info, FlowKind::Release, true, "2.0.0")
            .await
            .unwrap_err(),
        GitflowError::TagConflict("2.0.0".into())
    );
    assert_eq!(env.out(&["rev-parse", "refs/tags/2.0.0"]), before);
    assert_eq!(env.branches().len(), 3);
}

#[tokio::test]
async fn missing_permanent_branches_and_unknown_flow_branches_are_explicit() {
    let env = env_with(|_| {}).await;
    assert_eq!(
        env.git
            .gitflow_plan(&env.info, FlowKind::Feature, false, "x")
            .await
            .unwrap_err(),
        GitflowError::DevelopMissing("develop".into())
    );
    let env = flow_env().await;
    assert_eq!(
        env.git
            .gitflow_plan(&env.info, FlowKind::Release, true, "9.9")
            .await
            .unwrap_err(),
        GitflowError::NoSuchBranch("release/9.9".into())
    );
    let view = env.git.gitflow_view(&env.info).await.unwrap();
    assert!(view.active.is_empty());
    start(&env, FlowKind::Feature, "a").await;
    let view = env.git.gitflow_view(&env.info).await.unwrap();
    assert_eq!(view.active.len(), 1);
    assert_eq!(view.active[0].kind, FlowKind::Feature);
    assert_eq!(view.active[0].name, "a");
    let _ = git_output(env.path(), &["status"]);
}

#[tokio::test]
async fn a_fast_forwardable_branch_still_gets_its_own_merge_commit() {
    let env = flow_env().await;
    start(&env, FlowKind::Release, "3.0.0").await;
    write(env.path(), "r.txt", "r\n");
    commit_all(env.path(), "release work");
    let plan = env
        .git
        .gitflow_plan(&env.info, FlowKind::Release, true, "3.0.0")
        .await
        .unwrap();
    env.git
        .gitflow_run(&env.info, &plan, true, None)
        .await
        .unwrap();
    assert_eq!(env.parents("main"), 2);
    assert_eq!(env.parents("develop"), 2);
}
