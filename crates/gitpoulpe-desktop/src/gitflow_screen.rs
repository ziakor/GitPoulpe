use gitpoulpe_core::{FlowKind, FlowOutcome, FlowPlan, FlowStep, GitflowConfig, GitflowView};
use rune::TextEdit;

use crate::engine::Command;
use crate::state::State;

pub struct GitflowState {
    pub open: bool,
    pub request: u64,
    pub view: Option<GitflowView>,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub done: Option<String>,
    pub selected: Option<String>,
    pub plan: Option<(FlowPlan, bool)>,
    pub save_confirm: Option<(GitflowConfig, bool)>,
    pub drafted: bool,
    pub name: TextEdit,
    pub main: TextEdit,
    pub develop: TextEdit,
    pub feature: TextEdit,
    pub release: TextEdit,
    pub hotfix: TextEdit,
    pub tag_prefix: TextEdit,
    pub version: u32,
}

impl Default for GitflowState {
    fn default() -> Self {
        Self {
            open: false,
            request: 0,
            view: None,
            loading: false,
            busy: false,
            error: None,
            done: None,
            selected: None,
            plan: None,
            save_confirm: None,
            drafted: false,
            name: TextEdit::new(""),
            main: TextEdit::new(""),
            develop: TextEdit::new(""),
            feature: TextEdit::new(""),
            release: TextEdit::new(""),
            hotfix: TextEdit::new(""),
            tag_prefix: TextEdit::new(""),
            version: 0,
        }
    }
}

impl GitflowState {
    fn draft(&self) -> GitflowConfig {
        GitflowConfig {
            main: self.main.text.trim().to_owned(),
            develop: self.develop.text.trim().to_owned(),
            feature: self.feature.text.trim().to_owned(),
            release: self.release.text.trim().to_owned(),
            hotfix: self.hotfix.text.trim().to_owned(),
            tag_prefix: self.tag_prefix.text.trim().to_owned(),
        }
    }

    fn load_draft(&mut self, config: &GitflowConfig) {
        self.main = TextEdit::new(&config.main);
        self.develop = TextEdit::new(&config.develop);
        self.feature = TextEdit::new(&config.feature);
        self.release = TextEdit::new(&config.release);
        self.hotfix = TextEdit::new(&config.hotfix);
        self.tag_prefix = TextEdit::new(&config.tag_prefix);
        self.drafted = true;
    }
}

impl State {
    pub fn open_gitflow(&mut self) {
        if self.repo.is_none() {
            return;
        }
        self.close_history();
        self.close_ignored();
        self.close_branches();
        self.close_tags();
        self.close_remotes();
        self.close_conflicts();
        self.close_stashes();
        self.close_reflog();
        self.close_undo();
        self.close_worktrees();
        self.close_hooks();
        self.close_lfs();
        self.close_submodules();
        self.close_pulls();
        self.close_file_history();
        self.profiles_open = false;
        self.gitflow = GitflowState {
            open: true,
            ..GitflowState::default()
        };
        self.reload_gitflow();
    }

    pub fn close_gitflow(&mut self) {
        self.gitflow.open = false;
    }

    pub fn reload_gitflow(&mut self) {
        self.gitflow.request += 1;
        self.gitflow.loading = true;
        self.gitflow.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::GitflowLoad {
                request: self.gitflow.request,
            });
        }
    }

    pub fn select_flow_branch(&mut self, branch: String) {
        self.gitflow.selected = Some(branch);
        self.gitflow.plan = None;
        self.gitflow.version += 1;
    }

    pub fn gitflow_preview(&mut self, kind: FlowKind, finish: bool) {
        if self.gitflow.busy {
            return;
        }
        let name = if finish {
            let selected = self.gitflow.selected.as_ref();
            let Some(active) = self
                .gitflow
                .view
                .as_ref()
                .and_then(|v| v.active.iter().find(|b| Some(&b.branch) == selected))
            else {
                return;
            };
            if active.kind != kind {
                return;
            }
            active.name.clone()
        } else {
            self.gitflow.name.text.trim().to_owned()
        };
        self.gitflow.request += 1;
        self.gitflow.error = None;
        self.gitflow.done = None;
        self.gitflow.plan = None;
        self.gitflow.save_confirm = None;
        self.gitflow.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::GitflowPlan {
                request: self.gitflow.request,
                kind,
                finish,
                name,
            });
        }
    }

    pub fn gitflow_dismiss(&mut self) {
        self.gitflow.plan = None;
        self.gitflow.save_confirm = None;
        self.gitflow.version += 1;
    }

    pub fn gitflow_confirm(&mut self) {
        if self.gitflow.busy {
            return;
        }
        let Some((plan, finish)) = self.gitflow.plan.clone() else {
            return;
        };
        self.gitflow.request += 1;
        self.gitflow.busy = true;
        self.gitflow.error = None;
        self.gitflow.version += 1;
        let profile = self.active_profile().cloned().map(Box::new);
        if let Some(engine) = &self.engine {
            engine.send(Command::GitflowRun {
                request: self.gitflow.request,
                plan: Box::new(plan),
                finish,
                profile,
            });
        }
    }

    pub fn gitflow_save_preview(&mut self) {
        let Some(view) = &self.gitflow.view else {
            return;
        };
        let config = self.gitflow.draft();
        let create_develop = !view.develop_exists;
        self.gitflow.save_confirm = Some((config, create_develop));
        self.gitflow.plan = None;
        self.gitflow.error = None;
        self.gitflow.done = None;
        self.gitflow.version += 1;
    }

    pub fn gitflow_save_confirm(&mut self) {
        if self.gitflow.busy {
            return;
        }
        let Some((config, create_develop)) = self.gitflow.save_confirm.clone() else {
            return;
        };
        self.gitflow.request += 1;
        self.gitflow.busy = true;
        self.gitflow.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::GitflowSave {
                request: self.gitflow.request,
                config: Box::new(config),
                create_develop,
            });
        }
    }

    pub fn apply_gitflow_loaded(
        &mut self,
        request: u64,
        result: Result<Box<GitflowView>, String>,
    ) -> bool {
        if request != self.gitflow.request {
            return false;
        }
        self.gitflow.loading = false;
        match result {
            Ok(view) => {
                if !self.gitflow.drafted {
                    self.gitflow.load_draft(&view.config);
                }
                if self
                    .gitflow
                    .selected
                    .as_ref()
                    .is_some_and(|s| !view.active.iter().any(|b| &b.branch == s))
                {
                    self.gitflow.selected = None;
                }
                self.gitflow.view = Some(*view);
            }
            Err(message) => self.gitflow.error = Some(message),
        }
        self.gitflow.version += 1;
        true
    }

    pub fn apply_gitflow_planned(
        &mut self,
        request: u64,
        result: Result<Box<FlowPlan>, String>,
    ) -> bool {
        if request != self.gitflow.request {
            return false;
        }
        match result {
            Ok(plan) => {
                let finish = plan
                    .steps
                    .iter()
                    .any(|s| matches!(s, FlowStep::DeleteBranch { .. }));
                self.gitflow.plan = Some((*plan, finish));
            }
            Err(message) => self.gitflow.error = Some(message),
        }
        self.gitflow.version += 1;
        true
    }

    pub fn apply_gitflow_finished(
        &mut self,
        request: u64,
        result: Result<FlowOutcome, String>,
    ) -> bool {
        if request != self.gitflow.request {
            return false;
        }
        self.gitflow.busy = false;
        self.gitflow.plan = None;
        match result {
            Ok(FlowOutcome::Started { branch }) => {
                self.gitflow.done = Some(format!("Branche {branch} créée et active."));
                self.gitflow.name = TextEdit::new("");
            }
            Ok(FlowOutcome::Finished { steps }) => {
                self.gitflow.done = Some(format!(
                    "Flux terminé : {} étape(s) exécutée(s).",
                    steps.len()
                ));
            }
            Ok(FlowOutcome::Interrupted {
                done,
                at,
                remaining,
                files,
            }) => {
                self.notice = Some(format!(
                    "Flux interrompu après {} étape(s) : {} a produit des conflits dans {} fichier(s). Résolvez-les, puis terminez le flux à nouveau : les {} étape(s) restantes seront reprises.",
                    done.len(),
                    at.describe(),
                    files.len(),
                    remaining.len()
                ));
                self.request_refresh(false);
                self.probe_operation();
                self.gitflow.drafted = true;
                self.reload_gitflow();
                self.open_conflicts();
                return true;
            }
            Err(message) => self.gitflow.error = Some(message),
        }
        self.request_refresh(false);
        self.gitflow.drafted = true;
        self.reload_gitflow();
        true
    }

    pub fn apply_gitflow_saved(
        &mut self,
        request: u64,
        result: Result<Vec<String>, String>,
    ) -> bool {
        if request != self.gitflow.request {
            return false;
        }
        self.gitflow.busy = false;
        self.gitflow.save_confirm = None;
        match result {
            Ok(effects) => {
                self.gitflow.done = Some(effects.join(" ; "));
                self.gitflow.drafted = false;
                self.request_refresh(false);
                self.reload_gitflow();
            }
            Err(message) => self.gitflow.error = Some(message),
        }
        self.gitflow.version += 1;
        true
    }
}
