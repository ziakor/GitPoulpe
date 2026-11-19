use std::path::{Path, PathBuf};
use std::process::Command;

use rune::{Key, KeyEvent, Mods, Runtime};

use super::{click_at, rect_of, runtime, step};
use crate::state::State;
use crate::terminal::{control_byte, key_bytes};

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}");
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn settle(rt: &mut Runtime<State>, what: &str, until: impl Fn(&mut Runtime<State>) -> bool) {
    for _ in 0..500 {
        step(rt);
        if until(rt) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("never saw {what}");
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn key(key: Key, mods: Mods) -> KeyEvent {
    KeyEvent { key, mods }
}

fn send(rt: &mut Runtime<State>, text: &str) {
    rt.state
        .terminal_key(&key(Key::Text(text.to_owned()), Mods::default()));
    rt.state.terminal_key(&key(Key::Enter, Mods::default()));
}

struct Setup {
    main: tempfile::TempDir,
    linked: PathBuf,
    _side: tempfile::TempDir,
}

fn repos() -> Setup {
    let main = tempfile::tempdir().unwrap();
    let p = main.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "a\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    let side = tempfile::tempdir().unwrap();
    let linked = side.path().join("second tree");
    git(
        p,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "other",
            linked.to_str().unwrap(),
        ],
    );
    Setup {
        main,
        linked,
        _side: side,
    }
}

fn app(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    rt
}

fn running(needle: &str) -> bool {
    std::fs::read_dir("/proc").unwrap().flatten().any(|entry| {
        std::fs::read(entry.path().join("cmdline"))
            .map(|raw| {
                String::from_utf8_lossy(&raw)
                    .replace('\0', " ")
                    .contains(needle)
            })
            .unwrap_or(false)
    })
}

fn unique(offset: u32) -> u32 {
    20_000 + (std::process::id() * 11 + offset * 17) % 40_000
}

fn text_of(rt: &Runtime<State>) -> String {
    rt.state
        .terminal
        .session
        .as_ref()
        .map(|s| s.snapshot().text())
        .unwrap_or_default()
}

#[test]
fn the_terminal_opens_in_the_selected_worktree_and_says_where() {
    let setup = repos();
    let mut rt = app(setup.main.path());
    rt.state.open_worktrees();
    settle(&mut rt, "the worktree list", |rt| {
        rt.state.worktrees.list.len() == 2 && !rt.state.worktrees.loading
    });
    click(&mut rt, "wt-terminal");
    step(&mut rt);
    assert!(!rt.state.terminal.open && rt.state.notice.is_some());
    rt.state.select_worktree(setup.linked.clone());
    step(&mut rt);
    click(&mut rt, "wt-terminal");
    step(&mut rt);
    assert!(rt.state.terminal.open);
    assert!(shows(&rt, &format!("{}", setup.linked.display())));
    send(&mut rt, "pwd -P");
    let expected = setup.linked.canonicalize().unwrap().display().to_string();
    settle(&mut rt, "the worktree path", |rt| {
        text_of(rt).contains(&expected)
    });
    assert!(shows(&rt, &expected));
}

#[test]
fn typing_interrupting_and_copying_work_through_the_screen() {
    let setup = repos();
    let mut rt = app(setup.main.path());
    rt.state.open_terminal(setup.main.path().to_path_buf());
    step(&mut rt);
    let command = format!("sleep {}", unique(1));
    send(&mut rt, &command);
    settle(&mut rt, "the command running", |_| running(&command));
    click(&mut rt, "tm-interrupt");
    settle(&mut rt, "the command stopped", |_| !running(&command));
    send(&mut rt, "echo typed-$((20+22))");
    settle(&mut rt, "the typed output on screen", |rt| {
        shows(rt, "typed-42")
    });
    let clipboard = rune::clipboard::memory();
    rt.use_clipboard(&clipboard);
    click(&mut rt, "tm-copy");
    settle(&mut rt, "the clipboard", |_| clipboard.text().is_some());
    assert!(clipboard.text().unwrap().contains("typed-42"));
    assert!(shows(&rt, "Écran du terminal copié."));
}

#[test]
fn closing_stops_every_command_and_reads_the_repository_again() {
    let setup = repos();
    let mut rt = app(setup.main.path());
    rt.state.open_terminal(setup.main.path().to_path_buf());
    step(&mut rt);
    let (background, foreground) = (
        format!("sleep {}", unique(2)),
        format!("sleep {}", unique(3)),
    );
    send(&mut rt, &format!("{background} &"));
    send(&mut rt, "echo changed > from-terminal.txt");
    send(&mut rt, &foreground);
    settle(&mut rt, "both commands", |_| {
        running(&background) && running(&foreground)
    });
    click(&mut rt, "tm-close");
    settle(&mut rt, "the commands gone", |_| {
        !running(&background) && !running(&foreground)
    });
    assert!(!rt.state.terminal.open && rt.state.terminal.session.is_none());
    settle(&mut rt, "the new file in the status", |rt| {
        rt.state.repo.as_ref().is_some_and(|r| {
            r.snapshot
                .entries
                .iter()
                .any(|e| e.path.ends_with("from-terminal.txt"))
        })
    });
}

#[test]
fn switching_repository_or_screen_never_leaves_a_session_behind() {
    let setup = repos();
    let mut rt = app(setup.main.path());
    rt.state.open_terminal(setup.main.path().to_path_buf());
    step(&mut rt);
    let command = format!("sleep {}", unique(4));
    send(&mut rt, &command);
    settle(&mut rt, "the command", |_| running(&command));
    rt.state.open_branches();
    step(&mut rt);
    assert!(!rt.state.terminal.open);
    settle(&mut rt, "the command stopped when leaving", |_| {
        !running(&command)
    });
}

#[test]
fn a_finished_shell_is_reported_and_a_bad_shell_is_an_error() {
    let setup = repos();
    let mut rt = app(setup.main.path());
    rt.state.open_terminal(setup.main.path().to_path_buf());
    step(&mut rt);
    send(&mut rt, "exit 4");
    settle(&mut rt, "the exit notice", |rt| {
        shows(rt, "Session terminée (code 4)")
    });
    rt.state.close_terminal();
    rt.state.tools.book.shell.program = "/definitely/not/a/shell".to_owned();
    rt.state.open_terminal(setup.main.path().to_path_buf());
    step(&mut rt);
    assert!(shows(&rt, "Terminal indisponible"));
    assert!(rt.state.terminal.session.is_none());
}

#[test]
fn keys_become_the_bytes_a_terminal_expects() {
    let plain = Mods::default();
    let ctrl = Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    };
    let bytes = |k: Key, m: Mods, app: bool| key_bytes(&key(k, m), app);
    assert_eq!(
        bytes(Key::Text("é".into()), plain, false),
        Some("é".as_bytes().to_vec())
    );
    assert_eq!(bytes(Key::Enter, plain, false), Some(b"\r".to_vec()));
    assert_eq!(bytes(Key::Backspace, plain, false), Some(vec![0x7f]));
    assert_eq!(bytes(Key::Tab, plain, false), Some(b"\t".to_vec()));
    assert_eq!(
        bytes(
            Key::Tab,
            Mods {
                shift: true,
                ..plain
            },
            false
        ),
        Some(b"\x1b[Z".to_vec())
    );
    assert_eq!(bytes(Key::Up, plain, false), Some(b"\x1b[A".to_vec()));
    assert_eq!(bytes(Key::Up, plain, true), Some(b"\x1bOA".to_vec()));
    assert_eq!(bytes(Key::Left, plain, false), Some(b"\x1b[D".to_vec()));
    assert_eq!(bytes(Key::Delete, plain, false), Some(b"\x1b[3~".to_vec()));
    assert_eq!(bytes(Key::PageUp, plain, false), Some(b"\x1b[5~".to_vec()));
    assert_eq!(
        bytes(Key::Function(5), plain, false),
        Some(b"\x1b[15~".to_vec())
    );
    assert_eq!(bytes(Key::Command('c'), ctrl, false), Some(vec![0x03]));
    assert_eq!(bytes(Key::Command('d'), ctrl, false), Some(vec![0x04]));
    assert_eq!(
        bytes(Key::Command('c'), Mods { alt: true, ..ctrl }, false),
        Some(vec![0x1b, 0x03])
    );
    assert_eq!(
        bytes(Key::Command('c'), Mods { cmd: true, ..plain }, false),
        None
    );
    assert_eq!(
        bytes(Key::Text("x".into()), Mods { alt: true, ..plain }, false),
        Some(vec![0x1b, b'x'])
    );
    assert_eq!(control_byte('['), Some(0x1b));
    assert_eq!(control_byte('1'), None);
}

fn press_with(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    step(rt);
}

#[test]
fn the_keyboard_goes_to_the_program_not_to_the_application_shortcuts() {
    let setup = repos();
    let mut rt = app(setup.main.path());
    rt.state.open_terminal(setup.main.path().to_path_buf());
    step(&mut rt);
    for ch in "echo r$((1+1))".chars() {
        press_with(&mut rt, Key::Text(ch.to_string()), Mods::default());
    }
    press_with(&mut rt, Key::Enter, Mods::default());
    settle(&mut rt, "the typed command output", |rt| {
        text_of(rt).contains("r2")
    });
    let ctrl = Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    };
    let command = format!("sleep {}", unique(5));
    for ch in command.chars() {
        press_with(&mut rt, Key::Text(ch.to_string()), Mods::default());
    }
    press_with(&mut rt, Key::Enter, Mods::default());
    settle(&mut rt, "the command", |_| running(&command));
    press_with(&mut rt, Key::Command('c'), ctrl);
    settle(&mut rt, "the interrupt", |_| !running(&command));
    let clipboard = rune::clipboard::memory();
    rt.use_clipboard(&clipboard);
    press_with(
        &mut rt,
        Key::Command('c'),
        Mods {
            shift: true,
            ..ctrl
        },
    );
    settle(&mut rt, "the copy", |_| clipboard.text().is_some());
    assert!(clipboard.text().unwrap().contains("r2"));
}

#[test]
fn escape_goes_to_the_program_and_does_not_close_the_terminal() {
    let setup = repos();
    let mut rt = app(setup.main.path());
    rt.state.open_terminal(setup.main.path().to_path_buf());
    step(&mut rt);
    press_with(&mut rt, Key::Escape, Mods::default());
    assert!(rt.state.terminal.open);
}
