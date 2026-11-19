use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::state::ViewKind;

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

#[test]
fn switching_views_keeps_a_current_file_even_when_the_visible_rows_do_not_change() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "init"]);
    std::fs::write(p.join("a.txt"), "one\ntwo\n").unwrap();
    git(p, &["add", "a.txt"]);
    std::fs::write(p.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    let mut state = super::opened(p);
    state.settle(3, Duration::from_secs(20));
    let total = state.layout.total as usize;
    state.on_visible(0..total);
    assert_eq!(state.current, Some(0));
    for view in [ViewKind::Staged, ViewKind::Unstaged, ViewKind::Staged] {
        state.set_view(view);
        assert_eq!(state.view, view);
        assert!(!state.files.is_empty());
        assert_eq!(state.current, Some(0), "{view:?}");
    }
}
