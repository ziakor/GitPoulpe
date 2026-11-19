use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use gitpoulpe_terminal::{Snapshot, TermColor, Terminal, TerminalError};

fn shell(dir: &Path) -> Terminal {
    Terminal::spawn(dir, "/bin/sh", &[], 24, 80, || {}).unwrap()
}

fn wait_for(terminal: &Terminal, what: &str, check: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let snapshot = terminal.snapshot();
        if check(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "never saw {what}:\n{}",
            snapshot.text()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn unique(offset: u32) -> u32 {
    10_000 + (std::process::id() * 7 + offset * 13) % 50_000
}

fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map(|stat| !stat.contains(") Z "))
        .unwrap_or(false)
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
            && alive(
                entry
                    .file_name()
                    .to_str()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0),
            )
    })
}

#[test]
fn the_shell_starts_in_the_chosen_directory_even_when_its_name_is_unusual() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("work tree 'one' é");
    std::fs::create_dir(&dir).unwrap();
    let terminal = shell(&dir);
    terminal.write(b"pwd -P\n");
    let expected = dir.canonicalize().unwrap().display().to_string();
    wait_for(&terminal, "the working directory", |s| {
        s.text().contains(&expected)
    });
    let other = base.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let second = shell(&other);
    second.write(b"pwd -P\n");
    let expected = other.canonicalize().unwrap().display().to_string();
    let seen = wait_for(&second, "the second directory", |s| {
        s.text().contains(&expected)
    });
    assert!(!seen.text().contains("work tree"));
}

#[test]
fn typed_text_is_echoed_and_accents_survive() {
    let dir = tempfile::tempdir().unwrap();
    let terminal = shell(dir.path());
    terminal.write("echo héllo$((1+1)) wörld\n".as_bytes());
    wait_for(&terminal, "the echoed text", |s| {
        s.plain.iter().any(|l| l.contains("héllo2 wörld"))
    });
}

#[test]
fn resizing_reaches_the_program_and_the_screen() {
    let dir = tempfile::tempdir().unwrap();
    let terminal = shell(dir.path());
    terminal.write(b"stty size\n");
    wait_for(&terminal, "24 80", |s| {
        s.plain.iter().any(|l| l.trim_end().ends_with("24 80"))
    });
    terminal.resize(10, 40);
    terminal.write(b"stty size\n");
    let seen = wait_for(&terminal, "10 40", |s| {
        s.plain.iter().any(|l| l.trim_end().ends_with("10 40"))
    });
    assert_eq!(seen.size, (10, 40));
    assert_eq!(seen.plain.len(), 10);
}

#[test]
fn interrupting_stops_the_running_command_and_the_shell_stays_usable() {
    let dir = tempfile::tempdir().unwrap();
    let terminal = shell(dir.path());
    let command = format!("sleep {}", unique(1));
    terminal.write(format!("{command}\n").as_bytes());
    let started = Instant::now();
    while !running(&command) {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the command never started"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    terminal.write(&[0x03]);
    let waited = Instant::now();
    while running(&command) {
        assert!(
            waited.elapsed() < Duration::from_secs(5),
            "the command was not interrupted"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    terminal.write(b"echo still-$((1+1))\n");
    wait_for(&terminal, "the shell answering", |s| {
        s.plain.iter().any(|l| l.contains("still-2"))
    });
}

#[test]
fn closing_leaves_no_command_behind_foreground_or_background() {
    let dir = tempfile::tempdir().unwrap();
    let terminal = shell(dir.path());
    let (background, foreground) = (
        format!("sleep {}", unique(2)),
        format!("sleep {}", unique(3)),
    );
    terminal.write(format!("{background} &\n{foreground}\n").as_bytes());
    let started = Instant::now();
    while !(running(&background) && running(&foreground)) {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the commands never started"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    terminal.close();
    let waited = Instant::now();
    while running(&background) || running(&foreground) {
        assert!(
            waited.elapsed() < Duration::from_secs(5),
            "a command survived the close"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    terminal.write(b"echo ignored\n");
    terminal.close();
}

#[test]
fn dropping_the_terminal_also_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let command = format!("sleep {}", unique(4));
    {
        let terminal = shell(dir.path());
        terminal.write(format!("{command}\n").as_bytes());
        let started = Instant::now();
        while !running(&command) {
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let waited = Instant::now();
    while running(&command) {
        assert!(
            waited.elapsed() < Duration::from_secs(5),
            "the command survived the drop"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_exit_code_of_the_shell_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let terminal = shell(dir.path());
    terminal.write(b"exit 3\n");
    let seen = wait_for(&terminal, "the exit", |s| s.exited.is_some());
    assert_eq!(seen.exited, Some(3));
    assert!(seen.cursor.is_none());
}

#[test]
fn colours_are_kept_per_run_of_text() {
    let dir = tempfile::tempdir().unwrap();
    let terminal = shell(dir.path());
    terminal.write(b"printf '\\033[31mred\\033[0m plain \\033[1;38;2;1;2;3mtrue\\033[0m\\n'\n");
    let seen = wait_for(&terminal, "the coloured line", |s| {
        s.plain.iter().any(|l| l.contains("red plain true"))
    });
    let row = seen
        .rows
        .iter()
        .find(|r| r.iter().any(|run| run.text == "red"))
        .unwrap();
    let red = row.iter().find(|r| r.text == "red").unwrap();
    assert_eq!(red.fg, TermColor::Indexed(1));
    let plain = row.iter().find(|r| r.text.contains("plain")).unwrap();
    assert_eq!(plain.fg, TermColor::Default);
    let truecolor = row.iter().find(|r| r.text == "true").unwrap();
    assert_eq!(truecolor.fg, TermColor::Rgb(1, 2, 3));
    assert!(truecolor.bold);
}

#[test]
fn a_flood_of_output_is_coalesced_until_the_screen_is_read_and_stays_responsive() {
    let dir = tempfile::tempdir().unwrap();
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    let terminal = Terminal::spawn(dir.path(), "/bin/sh", &[], 24, 80, move || {
        counter.fetch_add(1, Ordering::Relaxed);
    })
    .unwrap();
    terminal.write(
        b"i=0; while [ $i -lt 20000 ]; do echo line$i; i=$((i+1)); done; echo flood-$((1+1))\n",
    );
    std::thread::sleep(Duration::from_millis(2500));
    assert!(
        wakes.load(Ordering::Relaxed) <= 2,
        "{} wake-ups for one unread screen",
        wakes.load(Ordering::Relaxed)
    );
    wait_for(&terminal, "the end of the flood", |s| {
        s.plain.iter().any(|l| l.contains("flood-2"))
    });
    terminal.write(b"echo after-$((2+2))\n");
    wait_for(&terminal, "a later command", |s| {
        s.plain.iter().any(|l| l.contains("after-4"))
    });
}

#[test]
fn a_missing_directory_or_shell_is_an_error_not_a_hang() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    assert!(matches!(
        Terminal::spawn(&missing, "/bin/sh", &[], 24, 80, || {}),
        Err(TerminalError::NoDirectory(_))
    ));
    assert!(matches!(
        Terminal::spawn(dir.path(), "/definitely/not/a/shell", &[], 24, 80, || {}),
        Err(TerminalError::Spawn(_))
    ));
}
