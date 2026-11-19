use gitpoulpe_core::{ToolBook, ToolConfig, ToolKind};
use gitpoulpe_git::{expand_args, split_args};
use rune::TextEdit;

use crate::engine::Command;
use crate::state::State;

pub const TOOLS_KEY: &str = "external_tools";

#[derive(Default)]
pub struct ToolForm {
    pub program: TextEdit,
    pub args: TextEdit,
}

#[derive(Default)]
pub struct ToolsState {
    pub open: bool,
    pub book: ToolBook,
    pub editor: ToolForm,
    pub diff: ToolForm,
    pub merge: ToolForm,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub running: Option<String>,
    pub status_error: Option<String>,
    pub status_notice: Option<String>,
    pub request: u64,
    pub version: u32,
}

impl ToolsState {
    pub fn form(&self, kind: ToolKind) -> &ToolForm {
        match kind {
            ToolKind::Editor => &self.editor,
            ToolKind::Diff => &self.diff,
            ToolKind::Merge => &self.merge,
        }
    }

    pub fn form_mut(&mut self, kind: ToolKind) -> &mut ToolForm {
        match kind {
            ToolKind::Editor => &mut self.editor,
            ToolKind::Diff => &mut self.diff,
            ToolKind::Merge => &mut self.merge,
        }
    }
}

fn validate(kind: ToolKind, config: &ToolConfig) -> Result<(), String> {
    if !config.configured() {
        return Ok(());
    }
    split_args(&config.args).map_err(|e| format!("{} : {e}", kind.caption()))?;
    let probe: Vec<(&str, &str)> = kind.variables().iter().map(|v| (*v, "x")).collect();
    expand_args(&config.args, &probe).map_err(|e| format!("{} : {e}", kind.caption()))?;
    Ok(())
}

impl State {
    pub fn open_tools_config(&mut self) {
        let book = self.tools.book.clone();
        for kind in ToolKind::ALL {
            let config = book.get(kind);
            let form = self.tools.form_mut(kind);
            form.program = TextEdit::new(config.program.clone());
            form.args = TextEdit::new(config.args.clone());
        }
        self.tools.open = true;
        self.tools.error = None;
        self.tools.notice = None;
        self.tools.version += 1;
    }

    pub fn close_tools(&mut self) {
        self.tools.open = false;
    }

    pub fn save_tools_config(&mut self) {
        let mut book = ToolBook::default();
        for kind in ToolKind::ALL {
            let form = self.tools.form(kind);
            let config = ToolConfig {
                program: form.program.text.trim().to_owned(),
                args: form.args.text.trim().to_owned(),
            };
            if let Err(message) = validate(kind, &config) {
                self.tools.error = Some(message);
                self.tools.notice = None;
                self.tools.version += 1;
                return;
            }
            book.set(kind, config);
        }
        let _ = self.prefs.set(TOOLS_KEY, &book);
        self.tools.book = book;
        self.tools.error = None;
        self.tools.notice = Some("Outils enregistrés.".to_owned());
        self.tools.version += 1;
    }

    fn tool_ready(&mut self, kind: ToolKind) -> Option<ToolConfig> {
        if self.tools.running.is_some() {
            return None;
        }
        self.tools.status_error = None;
        self.tools.status_notice = None;
        let config = self.tools.book.get(kind).clone();
        if !config.configured() {
            self.tools.status_error = Some(format!(
                "{} non configuré : ouvrez « Outils… » pour le renseigner.",
                kind.caption()
            ));
            self.tools.version += 1;
            return None;
        }
        Some(config)
    }

    fn start_tool(&mut self, kind: ToolKind, command: impl FnOnce(u64) -> Command) {
        self.tools.request += 1;
        self.tools.running = Some(kind.caption().to_owned());
        self.tools.version += 1;
        let request = self.tools.request;
        if let Some(engine) = &self.engine {
            engine.send(command(request));
        }
    }

    pub fn tool_open_editor(&mut self, path: String) {
        let Some(config) = self.tool_ready(ToolKind::Editor) else {
            return;
        };
        self.start_tool(ToolKind::Editor, |request| Command::ToolEditor {
            request,
            program: config.program,
            args: config.args,
            path,
        });
    }

    pub fn tool_open_diff(&mut self, oid: Option<String>, path: String, old_path: Option<String>) {
        let Some(config) = self.tool_ready(ToolKind::Diff) else {
            return;
        };
        self.start_tool(ToolKind::Diff, |request| Command::ToolDiff {
            request,
            program: config.program,
            args: config.args,
            oid,
            path,
            old_path,
        });
    }

    pub fn tool_open_merge(&mut self, path: String) {
        let Some(config) = self.tool_ready(ToolKind::Merge) else {
            return;
        };
        self.start_tool(ToolKind::Merge, |request| Command::ToolMerge {
            request,
            program: config.program,
            args: config.args,
            path,
        });
    }

    pub fn apply_tool_finished(&mut self, request: u64, result: Result<(), String>) -> bool {
        if request != self.tools.request || self.tools.running.is_none() {
            return false;
        }
        let label = self.tools.running.take().unwrap_or_default();
        match result {
            Ok(()) => self.tools.status_notice = Some(format!("{label} : session terminée.")),
            Err(message) => self.tools.status_error = Some(message),
        }
        self.tools.version += 1;
        self.request_refresh(false);
        if self.file_edit.open && !self.file_edit.dirty() {
            self.load_file_edit();
        }
        if self.conflicts.open {
            self.reload_conflicts();
        }
        true
    }
}
