use std::path::PathBuf;

use gitpoulpe_core::{
    FlowBranch, FlowKind, FlowOutcome, FlowPlan, FlowStep, GitflowConfig, GitflowView, MergeMode,
    MergeOutcome, Profile, SessionInfo,
};

use crate::Git;
use crate::branch::{BranchError, DeleteMode};
use crate::exec::ExecError;
use crate::merge::MergeError;
use crate::tag::TagError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitflowError {
    InvalidConfig(String),
    MainMissing(String),
    DevelopMissing(String),
    InvalidName,
    BranchExists(String),
    NoSuchBranch(String),
    OperationInProgress,
    LocalChanges(Vec<PathBuf>),
    TagConflict(String),
    PlanChanged,
    Branch(BranchError),
    Merge(MergeError),
    Tag(TagError),
    Exec(ExecError),
    Refused(String),
}

impl std::fmt::Display for GitflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(why) => write!(f, "configuration refusée : {why}"),
            Self::MainMissing(name) => write!(
                f,
                "la branche principale {name} n'existe pas : créez-la ou choisissez une autre branche"
            ),
            Self::DevelopMissing(name) => write!(
                f,
                "la branche de développement {name} n'existe pas : confirmez sa création à partir de la branche principale"
            ),
            Self::InvalidName => f.write_str("ce nom ne donne pas un nom de branche valide"),
            Self::BranchExists(name) => write!(f, "la branche {name} existe déjà"),
            Self::NoSuchBranch(name) => write!(f, "la branche {name} n'existe pas"),
            Self::OperationInProgress => f.write_str(
                "une autre opération Git est en cours : terminez-la ou abandonnez-la d'abord",
            ),
            Self::LocalChanges(files) => write!(
                f,
                "des modifications locales empêchent de changer de branche ({}) : enregistrez-les ou mettez-les de côté ; rien n'a été modifié",
                files
                    .iter()
                    .take(5)
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::TagConflict(name) => write!(
                f,
                "le tag {name} existe déjà sur un autre commit : il n'est jamais déplacé, choisissez un autre nom"
            ),
            Self::PlanChanged => f.write_str(
                "le dépôt a changé depuis l'annonce des étapes : rien n'a été exécuté, relisez le plan",
            ),
            Self::Branch(e) => write!(f, "{e}"),
            Self::Merge(e) => write!(f, "{e}"),
            Self::Tag(e) => write!(f, "{e}"),
            Self::Exec(e) => write!(f, "{e}"),
            Self::Refused(m) => write!(f, "Git refuse : {m}"),
        }
    }
}

impl std::error::Error for GitflowError {}

impl From<ExecError> for GitflowError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

impl From<BranchError> for GitflowError {
    fn from(error: BranchError) -> Self {
        Self::Branch(error)
    }
}

impl From<MergeError> for GitflowError {
    fn from(error: MergeError) -> Self {
        Self::Merge(error)
    }
}

impl From<TagError> for GitflowError {
    fn from(error: TagError) -> Self {
        Self::Tag(error)
    }
}

const KEYS: [&str; 6] = [
    "gitflow.branch.master",
    "gitflow.branch.develop",
    "gitflow.prefix.feature",
    "gitflow.prefix.release",
    "gitflow.prefix.hotfix",
    "gitflow.prefix.versiontag",
];

impl Git {
    async fn branch_oid(
        &self,
        info: &SessionInfo,
        name: &str,
    ) -> Result<Option<String>, GitflowError> {
        let spec = format!("refs/heads/{name}^{{commit}}");
        let out = self
            .exec(info, &["rev-parse", "--verify", "-q", &spec], &[], None)
            .await?;
        Ok(out.ok().then(|| out.text()))
    }

    async fn is_ancestor(
        &self,
        info: &SessionInfo,
        ancestor: &str,
        descendant: &str,
    ) -> Result<bool, GitflowError> {
        let out = self
            .exec(
                info,
                &["merge-base", "--is-ancestor", ancestor, descendant],
                &[],
                None,
            )
            .await?;
        match out.code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(GitflowError::Refused(out.stderr)),
        }
    }

    async fn current_branch_name(&self, info: &SessionInfo) -> Result<Option<String>, ExecError> {
        let out = self
            .exec(info, &["symbolic-ref", "--short", "-q", "HEAD"], &[], None)
            .await?;
        Ok(out.ok().then(|| out.text()))
    }

    pub async fn gitflow_view(&self, info: &SessionInfo) -> Result<GitflowView, GitflowError> {
        let out = self
            .exec(
                info,
                &["config", "--null", "--get-regexp", r"^gitflow\."],
                &[],
                None,
            )
            .await?;
        let mut config = GitflowConfig::default();
        let mut main_set = false;
        for record in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let text = String::from_utf8_lossy(record);
            let (key, value) = text.split_once('\n').unwrap_or((&text, ""));
            let value = value.to_owned();
            match key {
                "gitflow.branch.master" => {
                    config.main = value;
                    main_set = true;
                }
                "gitflow.branch.develop" => config.develop = value,
                "gitflow.prefix.feature" => config.feature = value,
                "gitflow.prefix.release" => config.release = value,
                "gitflow.prefix.hotfix" => config.hotfix = value,
                "gitflow.prefix.versiontag" => config.tag_prefix = value,
                _ => {}
            }
        }
        if !main_set
            && self.branch_oid(info, "main").await?.is_none()
            && self.branch_oid(info, "master").await?.is_some()
        {
            config.main = "master".to_owned();
        }
        let local = self
            .exec(
                info,
                &["config", "--local", "--null", "--get-regexp", r"^gitflow\."],
                &[],
                None,
            )
            .await?;
        let refs = self
            .exec_ok(
                info,
                &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
            )
            .await?
            .text();
        let mut active = Vec::new();
        for branch in refs.lines() {
            for kind in FlowKind::ALL {
                let prefix = config.prefix(kind);
                if let Some(name) = branch.strip_prefix(prefix)
                    && !prefix.is_empty()
                    && !name.is_empty()
                {
                    active.push(FlowBranch {
                        kind,
                        name: name.to_owned(),
                        branch: branch.to_owned(),
                    });
                }
            }
        }
        Ok(GitflowView {
            saved: local.ok() && !local.stdout.is_empty(),
            main_exists: self.branch_oid(info, &config.main).await?.is_some(),
            develop_exists: self.branch_oid(info, &config.develop).await?.is_some(),
            config,
            active,
        })
    }

    async fn validate_gitflow_config(
        &self,
        info: &SessionInfo,
        config: &GitflowConfig,
    ) -> Result<(), GitflowError> {
        let invalid = |why: &str| Err(GitflowError::InvalidConfig(why.to_owned()));
        for name in [&config.main, &config.develop] {
            if !self.valid_branch_name(info, name).await {
                return invalid(&format!("« {name} » n'est pas un nom de branche valide"));
            }
        }
        if config.main == config.develop {
            return invalid(
                "la branche principale et la branche de développement doivent différer",
            );
        }
        for kind in FlowKind::ALL {
            let prefix = config.prefix(kind);
            if !prefix.ends_with('/') || !self.valid_branch_name(info, &format!("{prefix}x")).await
            {
                return invalid(&format!(
                    "le préfixe {} « {prefix} » doit finir par « / » et former un nom de branche valide",
                    kind.caption()
                ));
            }
        }
        let prefixes = [&config.feature, &config.release, &config.hotfix];
        for (i, a) in prefixes.iter().enumerate() {
            for b in &prefixes[i + 1..] {
                if a.starts_with(b.as_str()) || b.starts_with(a.as_str()) {
                    return invalid(
                        "les préfixes feature, release et hotfix ne doivent pas se recouvrir",
                    );
                }
            }
        }
        for prefix in prefixes {
            for name in [&config.main, &config.develop] {
                if name.starts_with(prefix.as_str()) {
                    return invalid("une branche permanente ne peut pas porter un préfixe de flux");
                }
            }
        }
        if !config.tag_prefix.is_empty()
            && !self
                .valid_tag_name(info, &format!("{}1", config.tag_prefix))
                .await
        {
            return invalid("le préfixe de tag ne forme pas un nom de tag valide");
        }
        Ok(())
    }

    pub async fn gitflow_save(
        &self,
        info: &SessionInfo,
        config: &GitflowConfig,
        create_develop: bool,
    ) -> Result<Vec<String>, GitflowError> {
        self.validate_gitflow_config(info, config).await?;
        let Some(main_oid) = self.branch_oid(info, &config.main).await? else {
            return Err(GitflowError::MainMissing(config.main.clone()));
        };
        let mut effects = Vec::new();
        if self.branch_oid(info, &config.develop).await?.is_none() {
            if !create_develop {
                return Err(GitflowError::DevelopMissing(config.develop.clone()));
            }
            self.create_branch(info, &config.develop, Some(&main_oid))
                .await?;
            effects.push(format!(
                "branche {} créée à partir de {}",
                config.develop, config.main
            ));
        }
        let values = [
            &config.main,
            &config.develop,
            &config.feature,
            &config.release,
            &config.hotfix,
            &config.tag_prefix,
        ];
        for (key, value) in KEYS.iter().zip(values) {
            self.exec_ok(info, &["config", "--local", key, value])
                .await?;
        }
        effects.push("configuration enregistrée dans .git/config (clés gitflow.*)".to_owned());
        Ok(effects)
    }

    pub async fn gitflow_plan(
        &self,
        info: &SessionInfo,
        kind: FlowKind,
        finish: bool,
        name: &str,
    ) -> Result<FlowPlan, GitflowError> {
        let view = self.gitflow_view(info).await?;
        let config = &view.config;
        self.validate_gitflow_config(info, config).await?;
        if !view.main_exists {
            return Err(GitflowError::MainMissing(config.main.clone()));
        }
        if !view.develop_exists {
            return Err(GitflowError::DevelopMissing(config.develop.clone()));
        }
        let branch = config.branch(kind, name);
        if name.is_empty() || !self.valid_branch_name(info, &branch).await {
            return Err(GitflowError::InvalidName);
        }
        if kind.tagged()
            && !self
                .valid_tag_name(info, &format!("{}{name}", config.tag_prefix))
                .await
        {
            return Err(GitflowError::InvalidName);
        }
        let branch_oid = self.branch_oid(info, &branch).await?;
        let mut steps = Vec::new();
        let mut already_done = Vec::new();
        if !finish {
            if branch_oid.is_some() {
                return Err(GitflowError::BranchExists(branch));
            }
            steps.push(FlowStep::CreateBranch {
                name: branch.clone(),
                from: config.base(kind).to_owned(),
            });
            steps.push(FlowStep::Switch {
                branch: branch.clone(),
            });
            return Ok(FlowPlan {
                kind,
                name: name.to_owned(),
                branch,
                steps,
                already_done,
            });
        }
        if branch_oid.is_none() {
            return Err(GitflowError::NoSuchBranch(branch));
        }
        let mut current = self.current_branch_name(info).await?;
        let mut switch = |steps: &mut Vec<FlowStep>, target: &str| {
            if current.as_deref() != Some(target) {
                steps.push(FlowStep::Switch {
                    branch: target.to_owned(),
                });
                current = Some(target.to_owned());
            }
        };
        let targets: Vec<&str> = if kind.tagged() {
            vec![&config.main, &config.develop]
        } else {
            vec![&config.develop]
        };
        for target in targets {
            let merge = FlowStep::Merge {
                source: branch.clone(),
                target: target.to_owned(),
            };
            let merged = self.is_ancestor(info, &branch, target).await?;
            let tag = if kind.tagged() && target == config.main {
                let tag_name = format!("{}{name}", config.tag_prefix);
                let spec = format!("refs/tags/{tag_name}^{{commit}}");
                let existing = self
                    .exec(info, &["rev-parse", "--verify", "-q", &spec], &[], None)
                    .await?;
                let step = FlowStep::Tag {
                    name: tag_name.clone(),
                    target: target.to_owned(),
                };
                if existing.ok() {
                    if self.is_ancestor(info, &branch, &existing.text()).await? {
                        already_done.push(step);
                        None
                    } else {
                        return Err(GitflowError::TagConflict(tag_name));
                    }
                } else {
                    Some(step)
                }
            } else {
                None
            };
            if merged {
                already_done.push(merge.clone());
            }
            if !merged || tag.is_some() {
                switch(&mut steps, target);
            }
            if !merged {
                steps.push(merge);
            }
            steps.extend(tag);
        }
        switch(&mut steps, &config.develop);
        steps.push(FlowStep::DeleteBranch {
            name: branch.clone(),
        });
        Ok(FlowPlan {
            kind,
            name: name.to_owned(),
            branch,
            steps,
            already_done,
        })
    }

    pub async fn gitflow_run(
        &self,
        info: &SessionInfo,
        announced: &FlowPlan,
        finish: bool,
        profile: Option<&Profile>,
    ) -> Result<FlowOutcome, GitflowError> {
        if self
            .operation_state(info)
            .await
            .map_err(|e| GitflowError::Refused(e.to_string()))?
            .is_some()
        {
            return Err(GitflowError::OperationInProgress);
        }
        let fresh = self
            .gitflow_plan(info, announced.kind, finish, &announced.name)
            .await?;
        if fresh != *announced {
            return Err(GitflowError::PlanChanged);
        }
        if fresh
            .steps
            .iter()
            .any(|s| matches!(s, FlowStep::Switch { .. }))
        {
            let changes = self.tracked_changes(info).await?;
            if !changes.is_empty() {
                return Err(GitflowError::LocalChanges(changes));
            }
        }
        let mut done = Vec::new();
        let mut steps = fresh.steps.into_iter();
        while let Some(step) = steps.next() {
            match &step {
                FlowStep::CreateBranch { name, from } => {
                    let oid = self
                        .branch_oid(info, from)
                        .await?
                        .ok_or_else(|| GitflowError::NoSuchBranch(from.clone()))?;
                    self.create_branch(info, name, Some(&oid)).await?;
                }
                FlowStep::Switch { branch } => {
                    self.switch_branch(info, branch).await?;
                }
                FlowStep::Merge { source, target } => {
                    if self.current_branch_name(info).await?.as_deref() != Some(target) {
                        return Err(GitflowError::PlanChanged);
                    }
                    let head = self.exec_ok(info, &["rev-parse", "HEAD"]).await?.text();
                    let outcome = self
                        .merge_branch(
                            info,
                            source,
                            MergeMode::NoFastForward,
                            &head,
                            false,
                            profile,
                        )
                        .await?;
                    if let MergeOutcome::Conflicts { files } = outcome {
                        return Ok(FlowOutcome::Interrupted {
                            done,
                            at: step,
                            remaining: steps.collect(),
                            files,
                        });
                    }
                }
                FlowStep::Tag { name, target } => {
                    let oid = self
                        .branch_oid(info, target)
                        .await?
                        .ok_or_else(|| GitflowError::NoSuchBranch(target.clone()))?;
                    let label = match fresh.kind {
                        FlowKind::Hotfix => "Hotfix",
                        _ => "Release",
                    };
                    self.create_tag(
                        info,
                        name,
                        Some(&oid),
                        Some(&format!("{label} {}", fresh.name)),
                        profile,
                    )
                    .await?;
                }
                FlowStep::DeleteBranch { name } => {
                    self.delete_branch(info, name, &DeleteMode::Merged).await?;
                }
            }
            done.push(step);
        }
        Ok(if finish {
            FlowOutcome::Finished { steps: done }
        } else {
            FlowOutcome::Started {
                branch: fresh.branch,
            }
        })
    }
}
