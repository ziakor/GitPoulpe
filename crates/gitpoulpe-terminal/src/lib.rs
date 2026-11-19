use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

pub const DEFAULT_ROWS: u16 = 24;
pub const DEFAULT_COLS: u16 = 80;
const SCROLLBACK: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermColor {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl From<vt100::Color> for TermColor {
    fn from(color: vt100::Color) -> Self {
        match color {
            vt100::Color::Default => Self::Default,
            vt100::Color::Idx(index) => Self::Indexed(index),
            vt100::Color::Rgb(r, g, b) => Self::Rgb(r, g, b),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub fg: TermColor,
    pub bg: TermColor,
    pub bold: bool,
    pub inverse: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub rows: Vec<Vec<Run>>,
    pub plain: Vec<String>,
    pub cursor: Option<(u16, u16)>,
    pub size: (u16, u16),
    pub exited: Option<i32>,
}

impl Snapshot {
    pub fn text(&self) -> String {
        self.plain
            .iter()
            .map(|line| line.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_owned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalError {
    NoDirectory(String),
    Spawn(String),
    Pty(String),
}

impl std::fmt::Display for TerminalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDirectory(path) => write!(f, "le dossier {path} n'existe pas"),
            Self::Spawn(why) => write!(f, "impossible de lancer le shell : {why}"),
            Self::Pty(why) => write!(f, "impossible d'ouvrir le terminal : {why}"),
        }
    }
}

impl std::error::Error for TerminalError {}

struct Flags {
    finished: AtomicBool,
    pending: AtomicBool,
}

pub struct Terminal {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    parser: Arc<Mutex<vt100::Parser>>,
    flags: Arc<Flags>,
    pid: Option<u32>,
    closed: AtomicBool,
}

impl Terminal {
    pub fn spawn(
        directory: &Path,
        shell: &str,
        args: &[String],
        rows: u16,
        cols: u16,
        on_output: impl Fn() + Send + 'static,
    ) -> Result<Self, TerminalError> {
        if !directory.is_dir() {
            return Err(TerminalError::NoDirectory(directory.display().to_string()));
        }
        let rows = rows.max(2);
        let cols = cols.max(2);
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| TerminalError::Pty(e.to_string()))?;
        let mut command = CommandBuilder::new(shell);
        command.args(args);
        command.cwd(directory);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| TerminalError::Spawn(e.to_string()))?;
        drop(pair.slave);
        let pid = child.process_id();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| TerminalError::Pty(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| TerminalError::Pty(e.to_string()))?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            nz(rows),
            nz(cols),
            SCROLLBACK,
        )));
        let flags = Arc::new(Flags {
            finished: AtomicBool::new(false),
            pending: AtomicBool::new(false),
        });
        let (thread_parser, thread_flags) = (parser.clone(), flags.clone());
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut parser) = thread_parser.lock() {
                            parser.process(&buffer[..n]);
                        }
                        if !thread_flags.pending.swap(true, Ordering::AcqRel) {
                            on_output();
                        }
                    }
                }
            }
            thread_flags.finished.store(true, Ordering::Release);
            on_output();
        });
        Ok(Self {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            parser,
            flags,
            pid,
            closed: AtomicBool::new(false),
        })
    }

    pub fn write(&self, bytes: &[u8]) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.write_all(bytes);
            let _ = writer.flush();
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(2), cols.max(2));
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        if let Ok(mut parser) = self.parser.lock() {
            let (current_rows, current_cols) = parser.screen().size();
            if (current_rows.get(), current_cols.get()) == (rows, cols) {
                return;
            }
            parser.screen_mut().set_size(nz(rows), nz(cols));
        }
        if let Ok(master) = self.master.lock() {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    fn exit_code(&self) -> Option<i32> {
        let mut child = self.child.lock().ok()?;
        child
            .try_wait()
            .ok()
            .flatten()
            .map(|status| status.exit_code() as i32)
    }

    pub fn snapshot(&self) -> Snapshot {
        self.flags.pending.store(false, Ordering::Release);
        let exited =
            if self.flags.finished.load(Ordering::Acquire) || self.closed.load(Ordering::Acquire) {
                self.exit_code()
            } else {
                None
            };
        let parser = self
            .parser
            .lock()
            .expect("the terminal state is not poisoned");
        let screen = parser.screen();
        let (rows, cols) = {
            let (r, c) = screen.size();
            (r.get(), c.get())
        };
        let mut lines = Vec::with_capacity(rows as usize);
        let mut plain = Vec::with_capacity(rows as usize);
        for row in 0..rows {
            let mut runs: Vec<Run> = Vec::new();
            let mut line = String::new();
            for col in 0..cols {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let contents: String = if cell.has_contents() {
                    cell.contents().to_owned()
                } else {
                    " ".to_owned()
                };
                line.push_str(&contents);
                let (fg, bg) = (
                    TermColor::from(cell.fgcolor()),
                    TermColor::from(cell.bgcolor()),
                );
                match runs.last_mut() {
                    Some(last)
                        if last.fg == fg
                            && last.bg == bg
                            && last.bold == cell.bold()
                            && last.inverse == cell.inverse() =>
                    {
                        last.text.push_str(&contents);
                    }
                    _ => runs.push(Run {
                        text: contents,
                        fg,
                        bg,
                        bold: cell.bold(),
                        inverse: cell.inverse(),
                    }),
                }
            }
            lines.push(runs);
            plain.push(line);
        }
        let (cursor_row, cursor_col) = screen.cursor_position();
        Snapshot {
            rows: lines,
            plain,
            cursor: (!screen.hide_cursor() && exited.is_none()).then_some((cursor_row, cursor_col)),
            size: (rows, cols),
            exited,
        }
    }

    pub fn application_cursor(&self) -> bool {
        self.parser
            .lock()
            .map(|parser| parser.screen().application_cursor())
            .unwrap_or(false)
    }

    pub fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(pid) = self.pid {
            kill_session(pid);
        }
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.close();
    }
}

fn nz(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value.max(1)).expect("a size is never zero")
}

fn session_of(stat: &str) -> Option<u32> {
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(3)?.parse().ok()
}

fn kill_session(session: u32) {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        unsafe {
            libc::kill(-(session as i32), libc::SIGKILL);
        }
        return;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        if session_of(&stat) == Some(session) {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }
}
