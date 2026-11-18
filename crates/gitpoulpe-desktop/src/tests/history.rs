use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

use rune::Runtime;

use super::{click_at, rect_of, runtime, step};
use crate::history_state::HistoryStatus;
use crate::state::State;

fn git(dir: &Path, args: &[&str]) -> String {
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
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn branching_repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path();
    git(dir, &["init", "-q", "-b", "main"]);
    let commit = |name: &str| {
        std::fs::write(dir.join(format!("{}.txt", name.replace(' ', "_"))), name).unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", name]);
    };
    commit("root");
    for round in 0..3 {
        git(dir, &["checkout", "-q", "-b", &format!("feature-{round}")]);
        commit(&format!("feature {round}"));
        git(dir, &["checkout", "-q", "main"]);
        commit(&format!("main {round}"));
        git(
            dir,
            &[
                "merge",
                "-q",
                "--no-ff",
                "-m",
                &format!("merge feature {round}"),
                &format!("feature-{round}"),
            ],
        );
    }
    repo
}

fn big_history(dir: &Path, commits: usize) {
    git(dir, &["init", "-q", "-b", "main"]);
    let mut stream = String::new();
    for n in 1..=commits {
        stream.push_str(&format!(
            "commit refs/heads/main\nmark :{n}\ncommitter t <t@example.com> {} +0000\ndata 6\nc{n:05}\n",
            1_700_000_000 + n
        ));
        if n > 1 {
            stream.push_str(&format!("from :{}\n", n - 1));
        }
        stream.push_str(&format!("M 100644 inline f.txt\ndata 2\n{}\n\n", n % 10));
    }
    use std::io::Write;
    let mut child = Command::new("git")
        .current_dir(dir)
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stream.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    git(dir, &["reset", "-q", "--hard"]);
}

fn history_runtime(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    let button = rect_of(&rt, "history").expect("the top bar offers the history");
    click_at(
        &mut rt,
        button[0] + 10.0,
        button[1] + button[3] / 2.0,
        false,
    );
    for _ in 0..60 {
        step(&mut rt);
        if rt.state.history.status != HistoryStatus::Loading {
            break;
        }
    }
    for _ in 0..4 {
        step(&mut rt);
    }
    rt
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

#[test]
fn the_history_lists_the_commits_in_gits_order_with_the_graph_of_the_merges() {
    let repo = branching_repo();
    let rt = history_runtime(repo.path());
    assert!(rt.state.history_open);
    let expected = git(
        repo.path(),
        &[
            "log",
            "--topo-order",
            "--format=%H",
            "--branches",
            "--tags",
            "--remotes",
            "HEAD",
        ],
    );
    let got: Vec<String> = rt
        .state
        .history
        .commits
        .iter()
        .map(|c| c.oid.clone())
        .collect();
    assert_eq!(got, expected.lines().map(str::to_owned).collect::<Vec<_>>());
    assert!(rt.state.history.done, "a short history is read in one page");
    assert!(
        shows(&rt, "merge feature 2"),
        "the newest merge is on screen"
    );
    let glyphs: Vec<String> = rt
        .state
        .history
        .rows
        .iter()
        .map(|row| gitpoulpe_core::glyphs(row, rt.state.history.width))
        .collect();
    assert!(
        glyphs.iter().any(|g| g.contains('╮')),
        "a merge opens a lane: {glyphs:?}"
    );
    assert!(
        glyphs.iter().any(|g| g.contains('╯')),
        "a fork point closes it: {glyphs:?}"
    );
    assert!(rt.state.history.width >= 2);
}

#[test]
fn a_long_history_is_read_progressively_while_scrolling_and_selecting_does_not_block_it() {
    let repo = tempfile::tempdir().unwrap();
    big_history(repo.path(), 3_000);
    let mut rt = history_runtime(repo.path());
    assert_eq!(
        rt.state.history.commits.len(),
        200,
        "only the first page was read"
    );
    assert!(!rt.state.history.done);
    assert!(
        (1..120).contains(&super::mounted(&rt, "hk")),
        "rendering stays bounded: {}",
        super::mounted(&rt, "hk")
    );

    let row = rect_of(&rt, "hk3").expect("a row is on screen");
    click_at(&mut rt, row[0] + 200.0, row[1] + row[3] / 2.0, false);
    for _ in 0..3 {
        step(&mut rt);
    }
    assert_eq!(rt.state.history.selected, Some(3));
    assert!(
        shows(&rt, &format!("Commit {}", rt.state.history.commits[3].oid)),
        "the detail panel shows it"
    );

    let mut last = rt.state.history.commits.len();
    for _ in 0..40 {
        rt.wheel(500.0, 300.0, -3_000.0);
        for _ in 0..3 {
            step(&mut rt);
        }
        if rt.state.history.commits.len() > last {
            last = rt.state.history.commits.len();
        }
        if rt.state.history.done || last >= 600 {
            break;
        }
    }
    assert!(last >= 400, "scrolling pulled more pages: {last}");
    assert!(rt.state.history.commits.len() <= 3_000);
    assert_eq!(
        rt.state.history.selected,
        Some(3),
        "the selection survived the scrolling"
    );
    let sorted: Vec<&str> = rt
        .state
        .history
        .commits
        .iter()
        .take(5)
        .map(|c| c.subject.as_str())
        .collect();
    assert_eq!(sorted, ["c03000", "c02999", "c02998", "c02997", "c02996"]);
}

#[test]
fn a_repository_without_a_commit_and_a_git_error_have_distinct_states() {
    let empty = tempfile::tempdir().unwrap();
    git(empty.path(), &["init", "-q", "-b", "main"]);
    let rt = history_runtime(empty.path());
    assert_eq!(rt.state.history.status, HistoryStatus::NoCommits);
    assert!(shows(&rt, "Aucun commit"));
    assert!(!shows(&rt, "Lecture de l'historique impossible"));

    let broken = branching_repo();
    let mut state = super::opened(broken.path());
    let objects = broken.path().join(".git/objects");
    std::fs::set_permissions(&objects, std::fs::Permissions::from_mode(0o000)).unwrap();
    state.open_history();
    for _ in 0..200 {
        state.drain();
        if !matches!(state.history.status, HistoryStatus::Loading) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::fs::set_permissions(&objects, std::fs::Permissions::from_mode(0o755)).unwrap();
    let HistoryStatus::Failed(message) = state.history.status.clone() else {
        panic!(
            "a Git failure must not look like an empty repository: {:?}",
            state.history.status
        )
    };
    assert!(!message.is_empty());
    let mut rt = runtime(state);
    for _ in 0..4 {
        step(&mut rt);
    }
    assert!(shows(&rt, "Lecture de l'historique impossible"));
    assert!(!shows(&rt, "Aucun commit"));
}

#[test]
fn a_new_commit_reloads_the_open_history_and_escape_closes_it() {
    let repo = branching_repo();
    let mut rt = history_runtime(repo.path());
    let before = rt.state.history.commits.len();
    std::fs::write(repo.path().join("later.txt"), "later").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "a later commit"]);
    rt.state.refresh();
    for _ in 0..120 {
        step(&mut rt);
        if rt.state.history.commits.len() > before {
            break;
        }
    }
    assert_eq!(rt.state.history.commits.len(), before + 1);
    assert_eq!(rt.state.history.commits[0].subject, "a later commit");

    super::press(&mut rt, rune::Key::Escape);
    assert!(!rt.state.history_open);
}
