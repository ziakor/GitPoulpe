use std::path::PathBuf;
use std::sync::Arc;

use gitpoulpe_git::expand_args;
use gitpoulpe_terminal::{DEFAULT_COLS, DEFAULT_ROWS, Terminal};
use rune::{Key, KeyEvent, Tx};

use crate::engine::Event;
use crate::state::State;

pub const CELL_WIDTH: f32 = 7.2;
pub const CELL_HEIGHT: f32 = 16.0;

#[derive(Default)]
pub struct TerminalState {
    pub open: bool,
    pub session: Option<Arc<Terminal>>,
    pub directory: String,
    pub shell: String,
    pub error: Option<String>,
    pub copy_pending: bool,
    pub focus_pending: bool,
    pub notice: Option<String>,
    pub version: u32,
}

pub fn control_byte(c: char) -> Option<u8> {
    match c.to_ascii_lowercase() {
        c @ 'a'..='z' => Some(c as u8 - b'a' + 1),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        ' ' => Some(0),
        _ => None,
    }
}

fn function_key(n: u8) -> Option<Vec<u8>> {
    let code = match n {
        1 => return Some(b"\x1bOP".to_vec()),
        2 => return Some(b"\x1bOQ".to_vec()),
        3 => return Some(b"\x1bOR".to_vec()),
        4 => return Some(b"\x1bOS".to_vec()),
        5 => 15,
        6 => 17,
        7 => 18,
        8 => 19,
        9 => 20,
        10 => 21,
        11 => 23,
        12 => 24,
        _ => return None,
    };
    Some(format!("\x1b[{code}~").into_bytes())
}

pub fn key_bytes(event: &KeyEvent, application_cursor: bool) -> Option<Vec<u8>> {
    let arrow = |letter: u8| {
        if application_cursor {
            vec![0x1b, b'O', letter]
        } else {
            vec![0x1b, b'[', letter]
        }
    };
    let bytes = match &event.key {
        Key::Text(text) => {
            let mut bytes = Vec::new();
            if event.mods.alt {
                bytes.push(0x1b);
            }
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        Key::Command(c) if event.mods.ctrl => {
            let mut bytes = Vec::new();
            if event.mods.alt {
                bytes.push(0x1b);
            }
            bytes.push(control_byte(*c)?);
            bytes
        }
        Key::Command(_) => return None,
        Key::Enter => b"\r".to_vec(),
        Key::Backspace => vec![0x7f],
        Key::Delete => b"\x1b[3~".to_vec(),
        Key::Tab if event.mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Escape => vec![0x1b],
        Key::Up => arrow(b'A'),
        Key::Down => arrow(b'B'),
        Key::Right => arrow(b'C'),
        Key::Left => arrow(b'D'),
        Key::Home => b"\x1b[H".to_vec(),
        Key::End => b"\x1b[F".to_vec(),
        Key::PageUp => b"\x1b[5~".to_vec(),
        Key::PageDown => b"\x1b[6~".to_vec(),
        Key::Insert => b"\x1b[2~".to_vec(),
        Key::Function(n) => function_key(*n)?,
        _ => return None,
    };
    Some(bytes)
}

impl State {
    pub fn open_terminal(&mut self, directory: PathBuf) {
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
        self.close_shortcuts();
        self.close_workspaces();
        self.profiles_open = false;
        self.terminal = TerminalState::default();
        let config = self.tools.book.shell.clone();
        let shell = if config.configured() {
            config.program.trim().to_owned()
        } else {
            std::env::var("SHELL")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "/bin/sh".to_owned())
        };
        let shown = directory.to_string_lossy().into_owned();
        let args = match expand_args(&config.args, &[("repo", shown.as_str())]) {
            Ok(args) => args,
            Err(error) => {
                self.terminal.open = true;
                self.terminal.directory = shown;
                self.terminal.shell = shell;
                self.terminal.error = Some(format!("{error}"));
                return;
            }
        };
        self.terminal.open = true;
        self.terminal.directory = shown;
        self.terminal.shell = shell.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        let spawned = Terminal::spawn(
            &directory,
            &shell,
            &args,
            DEFAULT_ROWS,
            DEFAULT_COLS,
            move || {
                let _ = sender.send(Event::TerminalOutput);
                if let Some(wake) = waker.lock().unwrap().as_ref() {
                    wake();
                }
            },
        );
        match spawned {
            Ok(session) => {
                self.terminal.session = Some(Arc::new(session));
                self.terminal.focus_pending = true;
            }
            Err(error) => self.terminal.error = Some(error.to_string()),
        }
        self.terminal.version += 1;
    }

    pub fn worktree_open_terminal(&mut self) {
        match self.worktrees.selected.clone() {
            Some(path) => self.open_terminal(path),
            None => {
                self.notice =
                    Some("Sélectionnez d'abord le worktree où ouvrir le terminal.".to_owned());
            }
        }
    }

    pub fn close_terminal(&mut self) {
        let was_open = self.terminal.open;
        if let Some(session) = self.terminal.session.take() {
            session.close();
        }
        self.terminal.open = false;
        self.terminal.error = None;
        self.terminal.copy_pending = false;
        if was_open {
            self.request_refresh(false);
        }
    }

    pub fn terminal_key(&mut self, event: &KeyEvent) -> bool {
        let Some(session) = self.terminal.session.clone() else {
            return false;
        };
        if event.mods.ctrl && event.mods.shift && event.is_command('c') {
            self.terminal_copy();
            return true;
        }
        let Some(bytes) = key_bytes(event, session.application_cursor()) else {
            return false;
        };
        session.write(&bytes);
        true
    }

    pub fn terminal_interrupt(&mut self) {
        if let Some(session) = &self.terminal.session {
            session.write(&[0x03]);
        }
        self.terminal.focus_pending = true;
    }

    pub fn terminal_copy(&mut self) {
        self.terminal.copy_pending = true;
        self.terminal.focus_pending = true;
    }

    pub fn apply_terminal(&mut self, tx: &mut Tx) -> bool {
        let mut changed = false;
        if self.terminal.copy_pending {
            self.terminal.copy_pending = false;
            if let Some(session) = &self.terminal.session {
                let text = session.snapshot().text();
                tx.copy(text);
                self.terminal.notice = Some("Écran du terminal copié.".to_owned());
                changed = true;
            }
        }
        if self.terminal.focus_pending && self.terminal.open {
            self.terminal.focus_pending = false;
            tx.blur();
        }
        changed
    }

    pub fn apply_terminal_output(&mut self) -> bool {
        if !self.terminal.open {
            return false;
        }
        self.terminal.version += 1;
        true
    }
}
