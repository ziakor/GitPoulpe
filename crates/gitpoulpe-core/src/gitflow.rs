use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FlowKind {
    Feature,
    Release,
    Hotfix,
}

impl FlowKind {
    pub const ALL: [Self; 3] = [Self::Feature, Self::Release, Self::Hotfix];

    pub fn caption(self) -> &'static str {
        match self {
            Self::Feature => "feature",
            Self::Release => "release",
            Self::Hotfix => "hotfix",
        }
    }

    pub fn tagged(self) -> bool {
        !matches!(self, Self::Feature)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitflowConfig {
    pub main: String,
    pub develop: String,
    pub feature: String,
    pub release: String,
    pub hotfix: String,
    pub tag_prefix: String,
}

impl Default for GitflowConfig {
    fn default() -> Self {
        Self {
            main: "main".to_owned(),
            develop: "develop".to_owned(),
            feature: "feature/".to_owned(),
            release: "release/".to_owned(),
            hotfix: "hotfix/".to_owned(),
            tag_prefix: String::new(),
        }
    }
}

impl GitflowConfig {
    pub fn prefix(&self, kind: FlowKind) -> &str {
        match kind {
            FlowKind::Feature => &self.feature,
            FlowKind::Release => &self.release,
            FlowKind::Hotfix => &self.hotfix,
        }
    }

    pub fn branch(&self, kind: FlowKind, name: &str) -> String {
        format!("{}{name}", self.prefix(kind))
    }

    pub fn base(&self, kind: FlowKind) -> &str {
        match kind {
            FlowKind::Hotfix => &self.main,
            _ => &self.develop,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitflowView {
    pub config: GitflowConfig,
    pub saved: bool,
    pub main_exists: bool,
    pub develop_exists: bool,
    pub active: Vec<FlowBranch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowBranch {
    pub kind: FlowKind,
    pub name: String,
    pub branch: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowStep {
    CreateBranch { name: String, from: String },
    Switch { branch: String },
    Merge { source: String, target: String },
    Tag { name: String, target: String },
    DeleteBranch { name: String },
}

impl FlowStep {
    pub fn describe(&self) -> String {
        match self {
            Self::CreateBranch { name, from } => {
                format!("créer la branche {name} à partir de {from}")
            }
            Self::Switch { branch } => format!("passer sur {branch}"),
            Self::Merge { source, target } => {
                format!("fusionner {source} dans {target} (commit de fusion, sans avance rapide)")
            }
            Self::Tag { name, target } => format!("créer le tag annoté {name} sur {target}"),
            Self::DeleteBranch { name } => format!("supprimer la branche {name} (déjà fusionnée)"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowPlan {
    pub kind: FlowKind,
    pub name: String,
    pub branch: String,
    pub steps: Vec<FlowStep>,
    pub already_done: Vec<FlowStep>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowOutcome {
    Started {
        branch: String,
    },
    Finished {
        steps: Vec<FlowStep>,
    },
    Interrupted {
        done: Vec<FlowStep>,
        at: FlowStep,
        remaining: Vec<FlowStep>,
        files: Vec<PathBuf>,
    },
}
