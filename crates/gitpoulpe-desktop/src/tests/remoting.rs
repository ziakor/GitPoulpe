use std::path::Path;
use std::process::Command;

use rune::{Runtime, TextEdit};

use super::{click_at, press, rect_of, runtime, step};
use crate::state::State;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.org")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.org")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn shows(rt: &Runtime<State>, needle: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn settle(rt: &mut Runtime<State>, until: impl Fn(&State) -> bool) {
    for _ in 0..80 {
        step(rt);
        if until(&rt.state) {
            break;
        }
    }
    for _ in 0..4 {
        step(rt);
    }
    assert!(until(&rt.state), "the screen never settled");
}

fn idle(s: &State) -> bool {
    !s.remotes.busy && !s.remotes.loading
}

fn click(rt: &mut Runtime<State>, key: &str) {
    if key.starts_with("rm-") && !key.starts_with("rm-row") {
        let target = key.to_owned();
        rt.update(move |_, tx| tx.scroll_into_view_in("rm-panel", target));
        step(rt);
    }
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + 10.0, rect[1] + rect[3] / 2.0, false);
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.txt"), "one\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "first commit"]);
    git(
        p,
        &[
            "remote",
            "add",
            "origin",
            "https://user:s3cr3t@example.org/a.git",
        ],
    );
    git(p, &["remote", "add", "mirror", "git@example.org:b.git"]);
    let head = git(p, &["rev-parse", "HEAD"]);
    git(p, &["update-ref", "refs/remotes/mirror/main", &head]);
    git(p, &["config", "branch.main.remote", "mirror"]);
    git(p, &["config", "branch.main.merge", "refs/heads/main"]);
    dir
}

fn screen(dir: &Path) -> Runtime<State> {
    let mut rt = runtime(super::opened(dir));
    rt.on_key = Some(crate::keys::on_key);
    click(&mut rt, "branch-name");
    settle(&mut rt, |s| s.branches.open && !s.branches.loading);
    super::menu(&mut rt, "nav-remotes");
    settle(&mut rt, |s| s.remotes.open && idle(s));
    rt
}

fn pick(rt: &mut Runtime<State>, name: &str) {
    rt.state.select_remote(name);
    for _ in 0..6 {
        step(rt);
    }
}

fn type_into(rt: &mut Runtime<State>, set: impl FnOnce(&mut State)) {
    set(&mut rt.state);
    step(rt);
}

fn remotes(dir: &Path) -> Vec<String> {
    git(dir, &["remote"]).lines().map(str::to_owned).collect()
}

#[test]
fn the_remote_screen_lists_remotes_and_never_shows_credentials() {
    let dir = repo();
    let mut rt = screen(dir.path());
    let listed: Vec<String> = rt
        .state
        .remotes
        .remotes
        .iter()
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(listed, remotes(dir.path()));
    assert!(shows(&rt, "https://***@example.org/a.git"));
    pick(&mut rt, "origin");
    assert!(shows(&rt, "identifiants"));
    for element in rt.elements() {
        if let Some(text) = element.text.as_deref() {
            assert!(!text.contains("s3cr3t"), "credential leaked: {text}");
        }
    }
}

#[test]
fn a_remote_is_added_with_its_url_and_an_invalid_one_changes_nothing() {
    let dir = repo();
    let mut rt = screen(dir.path());
    let before = git(dir.path(), &["config", "--list", "--local"]);
    type_into(&mut rt, |s| {
        s.remotes.new_name = TextEdit::new("extra");
        s.remotes.new_url = TextEdit::new("ext::sh -c id");
    });
    click(&mut rt, "rm-add");
    settle(&mut rt, idle);
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("URL refusée"))
    );
    assert_eq!(git(dir.path(), &["config", "--list", "--local"]), before);
    type_into(&mut rt, |s| {
        s.remotes.new_url = TextEdit::new("https://example.org/extra.git");
    });
    click(&mut rt, "rm-add");
    settle(&mut rt, |s| idle(s) && s.remotes.remotes.len() == 3);
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "extra"]),
        "https://example.org/extra.git"
    );
}

#[test]
fn a_url_changed_behind_the_screen_is_not_overwritten() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "mirror");
    git(
        dir.path(),
        &["remote", "set-url", "mirror", "git@example.org:moved.git"],
    );
    type_into(&mut rt, |s| {
        s.remotes.url = TextEdit::new("git@example.org:mine.git")
    });
    click(&mut rt, "rm-url-save");
    settle(&mut rt, idle);
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("a changé"))
    );
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "mirror"]),
        "git@example.org:moved.git"
    );
    pick(&mut rt, "mirror");
    type_into(&mut rt, |s| {
        s.remotes.url = TextEdit::new("git@example.org:mine.git")
    });
    click(&mut rt, "rm-url-save");
    settle(&mut rt, idle);
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "mirror"]),
        "git@example.org:mine.git"
    );
}

#[test]
fn the_push_url_is_set_then_cleared() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "mirror");
    type_into(&mut rt, |s| {
        s.remotes.push_url = TextEdit::new("ssh://example.org/push.git")
    });
    click(&mut rt, "rm-push-save");
    settle(&mut rt, idle);
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "--push", "mirror"]),
        "ssh://example.org/push.git"
    );
    assert!(shows(&rt, "envoi différent"));
    pick(&mut rt, "mirror");
    click(&mut rt, "rm-push-clear");
    settle(&mut rt, idle);
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "--push", "mirror"]),
        "git@example.org:b.git"
    );
}

#[test]
fn renaming_keeps_the_tracking_branch_and_a_taken_name_is_refused() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "mirror");
    type_into(&mut rt, |s| s.remotes.rename = TextEdit::new("origin"));
    click(&mut rt, "rm-rename-save");
    settle(&mut rt, idle);
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("déjà ce nom"))
    );
    assert_eq!(remotes(dir.path()), ["mirror", "origin"]);
    pick(&mut rt, "mirror");
    type_into(&mut rt, |s| s.remotes.rename = TextEdit::new("backup"));
    click(&mut rt, "rm-rename-save");
    settle(&mut rt, |s| {
        idle(s) && s.remotes.remotes.iter().any(|r| r.name == "backup")
    });
    assert_eq!(git(dir.path(), &["config", "branch.main.remote"]), "backup");
    assert_eq!(
        git(dir.path(), &["rev-parse", "refs/remotes/backup/main"]),
        git(dir.path(), &["rev-parse", "HEAD"])
    );
}

#[test]
fn removal_states_its_impact_gives_the_restore_command_and_escape_cancels_first() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "mirror");
    click(&mut rt, "rm-remove");
    settle(&mut rt, |s| {
        matches!(&s.remotes.confirm_remove, Some((_, Some(_))))
    });
    assert!(shows(&rt, "perdra son suivi : main"));
    assert!(shows(
        &rt,
        "git remote add -- 'mirror' 'git@example.org:b.git'"
    ));
    press(&mut rt, rune::Key::Escape);
    assert!(rt.state.remotes.confirm_remove.is_none() && rt.state.remotes.open);
    assert!(remotes(dir.path()).contains(&"mirror".to_owned()));
    click(&mut rt, "rm-remove");
    settle(&mut rt, |s| {
        matches!(&s.remotes.confirm_remove, Some((_, Some(_))))
    });
    click(&mut rt, "rm-remove-confirm");
    settle(&mut rt, |s| idle(s) && s.remotes.remotes.len() == 1);
    assert_eq!(remotes(dir.path()), ["origin"]);
    assert_eq!(
        git(dir.path(), &["symbolic-ref", "--short", "HEAD"]),
        "main"
    );
    assert!(
        rt.state
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("Pour le rétablir"))
    );
}

#[test]
fn removing_a_remote_with_credentials_never_prints_them_in_the_restore_hint() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "origin");
    click(&mut rt, "rm-remove");
    settle(&mut rt, |s| {
        matches!(&s.remotes.confirm_remove, Some((_, Some(_))))
    });
    click(&mut rt, "rm-remove-confirm");
    settle(&mut rt, |s| idle(s) && s.remotes.remotes.len() == 1);
    let notice = rt.state.notice.clone().unwrap_or_default();
    assert!(!notice.contains("s3cr3t"), "{notice}");
    assert!(notice.contains("non affichée"));
}

#[test]
fn escape_closes_the_remote_screen() {
    let dir = repo();
    let mut rt = screen(dir.path());
    press(&mut rt, rune::Key::Escape);
    assert!(!rt.state.remotes.open);
}

#[test]
fn a_removal_confirmed_before_its_impact_is_known_does_nothing() {
    let dir = repo();
    let mut rt = screen(dir.path());
    pick(&mut rt, "mirror");
    rt.state.remotes.confirm_remove = Some(("mirror".to_owned(), None));
    rt.state.remote_remove();
    for _ in 0..6 {
        step(&mut rt);
    }
    assert!(!rt.state.remotes.busy);
    assert_eq!(remotes(dir.path()), ["mirror", "origin"]);
}

#[test]
fn a_token_used_as_the_user_name_is_masked_everywhere_too() {
    let dir = repo();
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "tokened",
            "https://ghp_t0kenvalue@github.com/o/r.git",
        ],
    );
    let mut rt = screen(dir.path());
    pick(&mut rt, "tokened");
    click(&mut rt, "rm-remove");
    settle(&mut rt, |s| {
        matches!(&s.remotes.confirm_remove, Some((_, Some(_))))
    });
    for element in rt.elements() {
        if let Some(text) = element.text.as_deref() {
            assert!(!text.contains("t0kenvalue"), "leaked on screen: {text}");
        }
    }
    click(&mut rt, "rm-remove-confirm");
    settle(&mut rt, |s| idle(s) && s.remotes.remotes.len() == 2);
    let notice = rt.state.notice.clone().unwrap_or_default();
    assert!(!notice.contains("t0kenvalue"), "{notice}");
}

mod access {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use std::time::Duration;

    use gitpoulpe_git::{Git, SshTools};

    use super::*;

    const FAKE_SSH: &str = include_str!("../../../gitpoulpe-git/tests/common/fake_ssh.sh");

    struct Net {
        dir: tempfile::TempDir,
        repo: tempfile::TempDir,
        _source: tempfile::TempDir,
    }

    impl Net {
        fn path(&self, name: &str) -> std::path::PathBuf {
            self.dir.path().join(name)
        }
        fn mode(&self, mode: &str) {
            std::fs::write(self.path("mode"), mode).unwrap();
        }
    }

    fn executable(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn setup() -> (Net, Runtime<State>) {
        let dir = tempfile::tempdir().unwrap();
        executable(&dir.path().join("fakessh"), FAKE_SSH);
        std::fs::write(dir.path().join("mode"), "ok").unwrap();
        let source = tempfile::tempdir().unwrap();
        git(source.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(source.path().join("a.txt"), "one\n").unwrap();
        git(source.path(), &["add", "-A"]);
        git(source.path(), &["commit", "-q", "-m", "first"]);
        git(source.path(), &["branch", "dev"]);
        git(source.path(), &["tag", "v1"]);
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(repo.path().join("b.txt"), "x\n").unwrap();
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "-q", "-m", "local"]);
        let url = format!("ssh://git@fakehost{}", source.path().display());
        git(repo.path(), &["remote", "add", "origin", &url]);
        let engine_git = Git::new()
            .with_env("GIT_SSH_COMMAND", dir.path().join("fakessh").as_os_str())
            .with_env("FAKESSH_DIR", dir.path().as_os_str())
            .with_ssh_tools(SshTools {
                keyscan: dir.path().join("fake-keyscan").into(),
                keygen: "ssh-keygen".into(),
                known_hosts: Some(dir.path().join("known_hosts")),
            });
        let mut state = State::new(None, rune::prefs::Prefs::memory());
        state.engine_git = Some(engine_git);
        state.setup(Arc::new(|| {}));
        state.open_blocking(repo.path().to_path_buf(), Duration::from_secs(20));
        let mut rt = runtime(state);
        rt.on_key = Some(crate::keys::on_key);
        click(&mut rt, "branch-name");
        settle(&mut rt, |s| s.branches.open && !s.branches.loading);
        super::super::menu(&mut rt, "nav-remotes");
        settle(&mut rt, |s| s.remotes.open && idle(s));
        pick(&mut rt, "origin");
        (
            Net {
                dir,
                repo,
                _source: source,
            },
            rt,
        )
    }

    fn finished(s: &State) -> bool {
        s.remotes
            .check
            .as_ref()
            .is_some_and(|c| !matches!(c.status, crate::remotes::CheckStatus::Running))
    }

    #[test]
    fn an_access_test_reads_the_remote_and_reports_what_it_found() {
        let (_net, mut rt) = setup();
        click(&mut rt, "rm-check");
        settle(&mut rt, finished);
        assert!(shows(
            &rt,
            "Accès réussi : 2 branche(s), 1 tag(s), HEAD sur main"
        ));
    }

    #[test]
    fn an_unknown_host_shows_its_fingerprint_and_is_trusted_only_on_explicit_request() {
        let (net, mut rt) = setup();
        net.mode("unknown");
        click(&mut rt, "rm-check");
        settle(&mut rt, finished);
        assert!(shows(&rt, "hôte SSH inconnu"));
        let key = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "", "-f"])
            .arg(net.path("hostkey"))
            .status()
            .unwrap();
        assert!(key.success());
        let public = std::fs::read_to_string(net.path("hostkey.pub")).unwrap();
        let mut parts = public.split_whitespace();
        let line = format!(
            "fakehost {} {}",
            parts.next().unwrap(),
            parts.next().unwrap()
        );
        executable(
            &net.path("fake-keyscan"),
            &format!("#!/bin/sh\ncat <<'EOF'\n{line}\nEOF\n"),
        );
        click(&mut rt, "rm-scan");
        settle(&mut rt, |s| {
            matches!(
                s.remotes.check.as_ref().and_then(|c| c.scan.as_ref()),
                Some(crate::remotes::ScanStatus::Keys(_))
            )
        });
        assert!(shows(&rt, "SHA256:"));
        assert!(!net.path("known_hosts").exists());
        net.mode("ok");
        click(&mut rt, "rm-trust0");
        settle(&mut rt, |s| {
            finished(s)
                && matches!(
                    s.remotes.check.as_ref().map(|c| &c.status),
                    Some(crate::remotes::CheckStatus::Reachable(_))
                )
        });
        assert_eq!(
            std::fs::read_to_string(net.path("known_hosts")).unwrap(),
            format!("{line}\n")
        );
        assert!(shows(&rt, "Accès réussi"));
    }

    #[test]
    fn a_changed_host_offers_no_way_to_trust_it() {
        let (net, mut rt) = setup();
        net.mode("changed");
        click(&mut rt, "rm-check");
        settle(&mut rt, finished);
        assert!(shows(&rt, "a changé"));
        assert!(shows(&rt, "ssh-keygen -R 'fakehost'"));
        assert!(rect_of(&rt, "rm-scan").is_none());
        assert!(rect_of(&rt, "rm-trust0").is_none());
    }

    #[test]
    fn a_running_test_can_be_cancelled_and_leaves_no_process_behind() {
        let (net, mut rt) = setup();
        net.mode("hang");
        click(&mut rt, "rm-check");
        for _ in 0..30 {
            step(&mut rt);
            if net.path("sleeper").exists() {
                break;
            }
        }
        assert!(net.path("sleeper").exists());
        assert!(shows(&rt, "Test de l'accès en cours"));
        click(&mut rt, "rm-check-cancel");
        settle(&mut rt, finished);
        assert!(shows(&rt, "opération annulée"));
        let pid = std::fs::read_to_string(net.path("sleeper"))
            .unwrap()
            .trim()
            .to_owned();
        let mut alive = true;
        for _ in 0..60 {
            alive = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map(|s| !s.contains(") Z"))
                .unwrap_or(false);
            if !alive {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive);
        let _ = &net.repo;
    }

    fn fetch_finished(s: &State) -> bool {
        s.remotes
            .fetch
            .as_ref()
            .is_some_and(|f| !matches!(f.status, crate::remotes::FetchStatus::Running { .. }))
    }

    #[test]
    fn a_fetch_updates_the_remote_references_and_leaves_the_work_alone() {
        let (net, mut rt) = setup();
        std::fs::write(net.repo.path().join("b.txt"), "edited\n").unwrap();
        let before = git(net.repo.path(), &["status", "--porcelain=v2", "--branch"]);
        click(&mut rt, "rm-fetch");
        settle(&mut rt, fetch_finished);
        assert!(shows(&rt, "Récupération terminée"));
        assert!(shows(&rt, "Votre travail local n'a pas été touché"));
        assert_eq!(
            git(net.repo.path(), &["status", "--porcelain=v2", "--branch"]),
            before
        );
        assert!(
            git(net.repo.path(), &["for-each-ref", "refs/remotes/origin"]).contains("origin/main")
        );
        assert_eq!(
            std::fs::read_to_string(net.repo.path().join("b.txt")).unwrap(),
            "edited\n"
        );
    }

    #[test]
    fn a_cancelled_fetch_says_so_and_can_be_launched_again() {
        let (net, mut rt) = setup();
        net.mode("hang");
        click(&mut rt, "rm-fetch");
        for _ in 0..30 {
            step(&mut rt);
            if net.path("sleeper").exists() {
                break;
            }
        }
        assert!(shows(&rt, "Récupération ·"));
        click(&mut rt, "rm-check-cancel");
        settle(&mut rt, fetch_finished);
        assert!(shows(&rt, "Récupération impossible : opération annulée"));
        assert!(shows(&rt, "intactes"));
        assert!(git(net.repo.path(), &["for-each-ref", "refs/remotes"]).is_empty());
        net.mode("ok");
        click(&mut rt, "rm-fetch");
        settle(&mut rt, |s| {
            fetch_finished(s)
                && matches!(
                    s.remotes.fetch.as_ref().map(|f| &f.status),
                    Some(crate::remotes::FetchStatus::Done(_))
                )
        });
        assert!(!git(net.repo.path(), &["for-each-ref", "refs/remotes"]).is_empty());
    }
}
