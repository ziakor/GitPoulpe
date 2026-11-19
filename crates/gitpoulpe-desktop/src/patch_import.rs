use std::path::PathBuf;
use std::sync::Arc;

use gitpoulpe_git::{ApplyMode, PATCH_LIMIT, PatchOutcome, PatchPreview};
use rune::Tx;

use crate::engine::Command;
use crate::state::State;

pub struct PatchState {
    pub open: bool,
    pub name: String,
    pub bytes: Option<Arc<Vec<u8>>>,
    pub preview: Option<PatchPreview>,
    pub mode: ApplyMode,
    pub three_way: bool,
    pub busy: bool,
    pub checked: Option<Result<(), String>>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub request: u64,
    pub version: u32,
}

impl Default for PatchState {
    fn default() -> Self {
        Self {
            open: false,
            name: String::new(),
            bytes: None,
            preview: None,
            mode: ApplyMode::IndexAndTree,
            three_way: false,
            busy: false,
            checked: None,
            error: None,
            notice: None,
            request: 0,
            version: 0,
        }
    }
}

pub fn mode_caption(mode: ApplyMode) -> &'static str {
    match mode {
        ApplyMode::WorkingTree => "dossier de travail seulement",
        ApplyMode::IndexAndTree => "index et dossier de travail",
        ApplyMode::IndexOnly => "index seulement",
    }
}

impl State {
    pub fn open_patch_import(&mut self) {
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
        self.close_gitflow();
        self.close_pulls();
        self.close_file_history();
        self.close_file_edit();
        self.close_tools();
        self.close_terminal();
        self.close_shortcuts();
        self.close_workspaces();
        self.close_favorites();
        self.close_overview();
        self.close_notices();
        self.close_issues();
        self.accounts.open = false;
        self.profiles_open = false;
        let request = self.patches.request + 1;
        self.patches = PatchState {
            open: true,
            request,
            ..PatchState::default()
        };
    }

    pub fn close_patch_import(&mut self) {
        self.patches.open = false;
        self.patches.busy = false;
        self.patches.request += 1;
    }

    pub fn patch_pick(&mut self, tx: &mut Tx) {
        let options = rune::FileDialogOptions {
            title: "Choisir un patch à importer".into(),
            ..Default::default()
        };
        tx.open_file(options, |s: &mut State, result, _| match result {
            Ok(Some(path)) => s.patch_load(path),
            Ok(None) => {}
            Err(error) => {
                s.patches.error = Some(format!("Sélecteur de fichier indisponible : {error}"));
                s.patches.version += 1;
            }
        });
    }

    pub fn patch_load(&mut self, path: PathBuf) {
        let patches = &mut self.patches;
        patches.error = None;
        patches.notice = None;
        patches.preview = None;
        patches.checked = None;
        patches.bytes = None;
        patches.name = path.display().to_string();
        patches.version += 1;
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size as usize > PATCH_LIMIT {
            patches.error = Some("patch de plus de 8 Mio : refusé".to_owned());
            return;
        }
        match std::fs::read(&path) {
            Ok(bytes) => {
                patches.bytes = Some(Arc::new(bytes));
                patches.request += 1;
                patches.busy = true;
                let request = patches.request;
                let patch = patches.bytes.clone().unwrap_or_default();
                if let Some(engine) = &self.engine {
                    engine.send(Command::PatchPreview { request, patch });
                }
            }
            Err(error) => patches.error = Some(format!("lecture impossible : {error}")),
        }
    }

    pub fn patch_set_mode(&mut self, mode: ApplyMode) {
        self.patches.mode = mode;
        self.patches.checked = None;
        self.patches.version += 1;
    }

    pub fn patch_toggle_three_way(&mut self) {
        self.patches.three_way = !self.patches.three_way;
        self.patches.checked = None;
        self.patches.version += 1;
    }

    pub fn patch_check(&mut self) {
        let Some(patch) = self.patches.bytes.clone() else {
            return;
        };
        if self.patches.busy {
            return;
        }
        self.patches.request += 1;
        self.patches.busy = true;
        self.patches.error = None;
        self.patches.notice = None;
        self.patches.version += 1;
        let (request, mode) = (self.patches.request, self.patches.mode);
        if let Some(engine) = &self.engine {
            engine.send(Command::PatchCheck {
                request,
                patch,
                mode,
            });
        }
    }

    pub fn patch_apply(&mut self) {
        let Some(patch) = self.patches.bytes.clone() else {
            return;
        };
        if self.patches.busy || self.patches.preview.is_none() {
            return;
        }
        if !self.patches.three_way && !matches!(self.patches.checked, Some(Ok(()))) {
            self.patches.error = Some(
                "Vérifiez d'abord le patch : il ne sera appliqué que s'il s'applique entièrement."
                    .to_owned(),
            );
            self.patches.version += 1;
            return;
        }
        self.patches.request += 1;
        self.patches.busy = true;
        self.patches.error = None;
        self.patches.notice = None;
        self.patches.version += 1;
        let (request, mode, three_way) = (
            self.patches.request,
            self.patches.mode,
            self.patches.three_way,
        );
        if let Some(engine) = &self.engine {
            engine.send(Command::PatchApply {
                request,
                patch,
                mode,
                three_way,
            });
        }
    }

    pub fn apply_patch_previewed(
        &mut self,
        request: u64,
        result: Result<PatchPreview, String>,
    ) -> bool {
        let patches = &mut self.patches;
        if !patches.open || request != patches.request {
            return false;
        }
        patches.busy = false;
        match result {
            Ok(preview) => patches.preview = Some(preview),
            Err(message) => patches.error = Some(message),
        }
        patches.version += 1;
        true
    }

    pub fn apply_patch_checked(&mut self, request: u64, result: Result<(), String>) -> bool {
        let patches = &mut self.patches;
        if !patches.open || request != patches.request {
            return false;
        }
        patches.busy = false;
        patches.checked = Some(result.clone());
        match result {
            Ok(()) => {
                patches.notice = Some("Le patch s'applique entièrement avec ce mode.".to_owned())
            }
            Err(message) => patches.error = Some(message),
        }
        patches.version += 1;
        true
    }

    pub fn apply_patch_applied(
        &mut self,
        request: u64,
        result: Result<PatchOutcome, String>,
    ) -> bool {
        let patches = &mut self.patches;
        if !patches.open || request != patches.request {
            return false;
        }
        patches.busy = false;
        let mode = mode_caption(patches.mode);
        match result {
            Ok(PatchOutcome::Applied(files)) => {
                patches.notice = Some(format!(
                    "Patch appliqué ({mode}) : {} fichier(s) modifié(s).",
                    files.len()
                ));
                patches.checked = None;
            }
            Ok(PatchOutcome::Conflicts(files)) => {
                patches.notice = Some(format!(
                    "Patch appliqué avec conflits dans {} : les marqueurs sont dans les fichiers, résolvez-les puis indexez.",
                    files.join(", ")
                ));
                patches.checked = None;
            }
            Err(message) => patches.error = Some(message),
        }
        patches.version += 1;
        let applied = patches.error.is_none();
        if applied {
            self.request_refresh(false);
        }
        true
    }
}
