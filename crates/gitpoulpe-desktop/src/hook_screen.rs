use gitpoulpe_core::HooksView;
use gitpoulpe_git::HookError;

use crate::engine::Command;
use crate::state::State;

#[derive(Default)]
pub struct HooksState {
    pub open: bool,
    pub request: u64,
    pub op_request: u64,
    pub view: Option<HooksView>,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub selected: Option<String>,
    pub confirm_shared: Option<(String, bool)>,
    pub version: u32,
}

impl State {
    pub fn open_hooks(&mut self) {
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
        self.close_lfs();
        self.close_submodules();
        self.close_gitflow();
        self.close_pulls();
        self.close_file_history();
        self.close_file_edit();
        self.close_tools();
        self.close_terminal();
        self.close_shortcuts();
        self.close_workspaces();
        self.profiles_open = false;
        self.hooks.open = true;
        self.reload_hooks();
    }

    pub fn close_hooks(&mut self) {
        self.hooks.open = false;
        self.hooks.confirm_shared = None;
    }

    pub fn reload_hooks(&mut self) {
        self.hooks.request += 1;
        self.hooks.loading = true;
        self.hooks.error = None;
        self.hooks.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::HookList {
                request: self.hooks.request,
            });
        }
    }

    pub fn select_hook(&mut self, name: &str) {
        self.hooks.selected = Some(name.to_owned());
        self.hooks.confirm_shared = None;
        self.hooks.version += 1;
    }

    pub fn hook_toggle(&mut self, confirmed: bool) {
        if self.hooks.busy {
            return;
        }
        let Some(name) = self.hooks.selected.clone() else {
            return;
        };
        let Some(hook) = self
            .hooks
            .view
            .as_ref()
            .and_then(|v| v.hooks.iter().find(|h| h.name == name))
        else {
            return;
        };
        let enabled = !hook.executable;
        self.hooks.busy = true;
        self.hooks.op_request += 1;
        self.hooks.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::HookSet {
                request: self.hooks.op_request,
                name,
                enabled,
                confirm_shared: confirmed,
            });
        }
    }

    pub fn hook_cancel_shared(&mut self) {
        self.hooks.confirm_shared = None;
        self.hooks.version += 1;
    }

    pub fn apply_hooks_listed(
        &mut self,
        request: u64,
        result: Result<Box<HooksView>, String>,
    ) -> bool {
        if request != self.hooks.request {
            return false;
        }
        self.hooks.loading = false;
        match result {
            Ok(view) => {
                if self
                    .hooks
                    .selected
                    .as_ref()
                    .is_some_and(|n| !view.hooks.iter().any(|h| &h.name == n))
                {
                    self.hooks.selected = None;
                }
                self.hooks.view = Some(*view);
                self.hooks.error = None;
            }
            Err(message) => self.hooks.error = Some(message),
        }
        self.hooks.version += 1;
        true
    }

    pub fn apply_hook_changed(
        &mut self,
        request: u64,
        name: String,
        enabled: bool,
        result: Result<Box<HooksView>, HookError>,
    ) -> bool {
        if request != self.hooks.op_request {
            return false;
        }
        self.hooks.busy = false;
        match result {
            Ok(view) => {
                self.hooks.view = Some(*view);
                self.hooks.confirm_shared = None;
                self.notice = Some(format!(
                    "Hook {name} {}.",
                    if enabled { "activé" } else { "désactivé" }
                ));
            }
            Err(HookError::SharedNeedsConfirmation(_)) => {
                self.hooks.confirm_shared = Some((name, enabled));
            }
            Err(error) => self.notice = Some(format!("Hook non modifié : {error}")),
        }
        self.hooks.version += 1;
        true
    }
}
