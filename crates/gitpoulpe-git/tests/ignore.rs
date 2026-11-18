mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{commit_all, init_repo, tree_fingerprint, write};
use gitpoulpe_core::{IgnoreScope, IgnoreSource, SessionInfo, rule_for_path};
use gitpoulpe_git::{Cancel, Git, IgnoreError};

struct Fixture {
    repo: tempfile::TempDir,
    _home: tempfile::TempDir,
    global: PathBuf,
    git: Git,
    info: SessionInfo,
}

fn run(dir: &Path, global: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", global)
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap()
}

async fn fixture() -> Fixture {
    let home = tempfile::tempdir().unwrap();
    let global = home.path().join("global-ignore");
    let config = home.path().join("gitconfig");
    std::fs::write(
        &config,
        format!("[core]\n\texcludesFile = {}\n", global.display()),
    )
    .unwrap();
    let repo = init_repo();
    let git = Git::new()
        .with_env("GIT_CONFIG_GLOBAL", &config)
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null");
    let info = git.open(repo.path(), &Cancel::new()).await.unwrap();
    Fixture {
        repo,
        _home: home,
        global,
        git,
        info,
    }
}

impl Fixture {
    fn path(&self) -> &Path {
        self.repo.path()
    }
    fn config(&self) -> PathBuf {
        self._home.path().join("gitconfig")
    }
    fn git_out(&self, args: &[&str]) -> String {
        let out = run(self.path(), &self.config(), args);
        assert!(out.status.success(), "{args:?}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
    fn ignored_by_git(&self, path: &str) -> bool {
        run(
            self.path(),
            &self.config(),
            &["check-ignore", "-q", "--", path],
        )
        .status
        .success()
    }
}

async fn layered() -> Fixture {
    let f = fixture().await;
    write(f.path(), ".gitignore", "*.log\nbuild/\n!keep.log\n");
    write(f.path(), "sub/.gitignore", "*.tmp\n!important.tmp\n");
    write(f.path(), ".git/info/exclude", "# local\nlocal.txt\n");
    std::fs::write(&f.global, "global.txt\n").unwrap();
    f
}

#[tokio::test]
async fn an_excluded_file_names_the_rule_the_line_and_the_scope_that_exclude_it() {
    let f = layered().await;
    for name in [
        "a.log",
        "keep.log",
        "build/out.bin",
        "sub/x.tmp",
        "sub/important.tmp",
        "local.txt",
        "global.txt",
        "plain.txt",
    ] {
        write(f.path(), name, "x\n");
    }
    let reason = |p: &'static str| {
        let f = &f;
        async move { f.git.ignore_reason(&f.info, Path::new(p)).await.unwrap() }
    };
    let r = reason("a.log").await.unwrap();
    assert_eq!(
        r.source,
        IgnoreSource::Scope(IgnoreScope::Directory("".into()))
    );
    assert_eq!((r.line, r.pattern.as_str()), (1, "*.log"));
    assert!(r.excludes() && !r.negated && !r.tracked);

    let r = reason("keep.log").await.unwrap();
    assert_eq!((r.line, r.pattern.as_str()), (3, "!keep.log"));
    assert!(
        r.negated && !r.excludes(),
        "an exception is not an exclusion"
    );

    let r = reason("build/out.bin").await.unwrap();
    assert_eq!((r.line, r.pattern.as_str()), (2, "build/"));

    let r = reason("sub/x.tmp").await.unwrap();
    assert_eq!(
        r.source,
        IgnoreSource::Scope(IgnoreScope::Directory("sub".into()))
    );
    assert_eq!((r.line, r.pattern.as_str()), (1, "*.tmp"));

    let r = reason("sub/important.tmp").await.unwrap();
    assert_eq!(
        r.source,
        IgnoreSource::Scope(IgnoreScope::Directory("sub".into()))
    );
    assert!(r.negated && !r.excludes());

    let r = reason("local.txt").await.unwrap();
    assert_eq!(r.source, IgnoreSource::Scope(IgnoreScope::Exclude));
    assert_eq!((r.line, r.pattern.as_str()), (2, "local.txt"));

    let r = reason("global.txt").await.unwrap();
    assert_eq!(r.source, IgnoreSource::Scope(IgnoreScope::Global));
    assert_eq!((r.line, r.pattern.as_str()), (1, "global.txt"));

    assert!(reason("plain.txt").await.is_none());
}

#[tokio::test]
async fn the_ignored_list_is_what_git_itself_reports() {
    let f = layered().await;
    for name in [
        "a.log",
        "keep.log",
        "build/one.bin",
        "build/deep/two.bin",
        "sub/x.tmp",
        "sub/important.tmp",
        "local.txt",
        "global.txt",
        "plain.txt",
    ] {
        write(f.path(), name, "x\n");
    }
    let listed = f.git.list_ignored(&f.info).await.unwrap();
    assert!(!listed.truncated);
    let oracle: Vec<PathBuf> = f
        .git_out(&["status", "--porcelain", "--ignored", "-uall"])
        .lines()
        .filter_map(|l| l.strip_prefix("!! "))
        .map(PathBuf::from)
        .collect();
    let mut oracle_sorted = oracle.clone();
    oracle_sorted.sort();
    assert!(!oracle.is_empty());
    let expand = |paths: &[PathBuf]| -> Vec<PathBuf> {
        let mut all = Vec::new();
        for p in paths {
            let full = f.path().join(p);
            if full.is_dir() {
                let mut stack = vec![full];
                while let Some(d) = stack.pop() {
                    for e in std::fs::read_dir(&d).unwrap() {
                        let e = e.unwrap().path();
                        if e.is_dir() {
                            stack.push(e);
                        } else {
                            all.push(e.strip_prefix(f.path()).unwrap().to_path_buf());
                        }
                    }
                }
            } else {
                all.push(p.clone());
            }
        }
        all.sort();
        all
    };
    assert!(
        listed.paths.iter().any(|p| p == Path::new("build/")),
        "a fully ignored directory is listed once"
    );
    assert_eq!(expand(&listed.paths), expand(&oracle_sorted));
    assert!(!listed.paths.iter().any(|p| p == Path::new("plain.txt")));
    assert!(!listed.paths.iter().any(|p| p == Path::new("keep.log")));
}

#[tokio::test]
async fn a_tracked_file_matching_a_rule_is_reported_as_tracked_not_as_ignored() {
    let f = fixture().await;
    write(f.path(), "tracked.log", "one\n");
    write(f.path(), "other.txt", "o\n");
    commit_all(f.path(), "base");
    write(f.path(), ".gitignore", "*.log\n");
    write(f.path(), "tracked.log", "two\n");
    let r = f
        .git
        .ignore_reason(&f.info, Path::new("tracked.log"))
        .await
        .unwrap()
        .unwrap();
    assert!(r.tracked && !r.excludes());
    assert_eq!(r.pattern, "*.log");
    assert!(!f.ignored_by_git("tracked.log") || r.tracked);

    let index = std::fs::read(f.path().join(".git/index")).unwrap();
    let files = f.git_out(&["ls-files"]);
    f.git
        .add_ignore_rule(&f.info, &IgnoreScope::Exclude, "/other.txt")
        .await
        .unwrap();
    f.git
        .add_ignore_rule(&f.info, &IgnoreScope::Directory("".into()), "/tracked.log")
        .await
        .unwrap();
    assert_eq!(f.git_out(&["ls-files"]), files, "the index is untouched");
    assert_eq!(std::fs::read(f.path().join(".git/index")).unwrap(), index);
    let status = f.git_out(&["status", "--porcelain"]);
    assert!(
        status.contains(" M tracked.log") && !status.contains("D "),
        "{status}"
    );
    let listed = f.git.list_ignored(&f.info).await.unwrap();
    assert!(!listed.paths.contains(&PathBuf::from("tracked.log")));
    assert!(!listed.paths.contains(&PathBuf::from("other.txt")));
}

#[tokio::test]
async fn adding_and_removing_a_rule_changes_only_the_chosen_file() {
    let f = layered().await;
    write(f.path(), "x.dat", "x\n");
    let others = |f: &Fixture| {
        (
            tree_fingerprint(f.path())
                .into_iter()
                .filter(|(p, _)| !p.starts_with(".git"))
                .collect::<Vec<_>>(),
            std::fs::read(&f.global).unwrap(),
        )
    };
    let before = others(&f);
    assert!(!f.ignored_by_git("x.dat"));

    f.git
        .add_ignore_rule(&f.info, &IgnoreScope::Exclude, "*.dat")
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.path().join(".git/info/exclude")).unwrap(),
        "# local\nlocal.txt\n*.dat\n"
    );
    assert_eq!(others(&f), before, "no other ignore file was touched");
    assert!(f.ignored_by_git("x.dat"));
    assert_eq!(
        f.git
            .add_ignore_rule(&f.info, &IgnoreScope::Exclude, "*.dat")
            .await,
        Err(IgnoreError::AlreadyPresent)
    );

    f.git
        .remove_ignore_rule(&f.info, &IgnoreScope::Exclude, "*.dat")
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.path().join(".git/info/exclude")).unwrap(),
        "# local\nlocal.txt\n",
        "comments and other rules survive"
    );
    assert_eq!(others(&f), before);
    assert!(!f.ignored_by_git("x.dat"));
    assert_eq!(
        f.git
            .remove_ignore_rule(&f.info, &IgnoreScope::Exclude, "*.dat")
            .await,
        Err(IgnoreError::NotFound)
    );
}

#[tokio::test]
async fn each_scope_writes_its_own_file_and_a_missing_file_is_created() {
    let f = layered().await;
    write(f.path(), "sub/deep/y.bak", "y\n");
    f.git
        .add_ignore_rule(&f.info, &IgnoreScope::Directory("sub/deep".into()), "*.bak")
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.path().join("sub/deep/.gitignore")).unwrap(),
        "*.bak\n"
    );
    assert!(f.ignored_by_git("sub/deep/y.bak"));
    assert_eq!(
        std::fs::read_to_string(f.path().join("sub/.gitignore")).unwrap(),
        "*.tmp\n!important.tmp\n"
    );

    f.git
        .add_ignore_rule(&f.info, &IgnoreScope::Global, "*.swp")
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&f.global).unwrap(),
        "global.txt\n*.swp\n"
    );
    let reason = f
        .git
        .ignore_reason(&f.info, Path::new("z.swp"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reason.source, IgnoreSource::Scope(IgnoreScope::Global));
    assert_eq!(
        std::fs::read_to_string(f.path().join(".gitignore")).unwrap(),
        "*.log\nbuild/\n!keep.log\n"
    );
}

#[tokio::test]
async fn existing_content_without_a_final_newline_and_with_crlf_is_kept() {
    let f = fixture().await;
    std::fs::write(f.path().join(".gitignore"), "a\r\nb").unwrap();
    f.git
        .add_ignore_rule(&f.info, &IgnoreScope::Directory("".into()), "c")
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(f.path().join(".gitignore")).unwrap(),
        b"a\r\nb\nc\n"
    );
    f.git
        .remove_ignore_rule(&f.info, &IgnoreScope::Directory("".into()), "a")
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(f.path().join(".gitignore")).unwrap(),
        b"b\nc\n"
    );
}

#[tokio::test]
async fn unsafe_scopes_and_rules_are_refused_before_any_write() {
    let f = fixture().await;
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), f.path().join("link")).unwrap();
    for scope in [
        IgnoreScope::Directory("../escape".into()),
        IgnoreScope::Directory("/abs".into()),
        IgnoreScope::Directory("link".into()),
        IgnoreScope::Directory("missing".into()),
    ] {
        assert!(
            f.git.add_ignore_rule(&f.info, &scope, "x").await.is_err(),
            "{scope:?}"
        );
    }
    assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    let scope = IgnoreScope::Exclude;
    for rule in ["", "  ", "a\nb", "# note", " lead", "a\0b"] {
        assert_eq!(
            f.git.add_ignore_rule(&f.info, &scope, rule).await,
            Err(IgnoreError::InvalidRule),
            "{rule:?}"
        );
    }
    assert!(
        f.git
            .ignore_reason(&f.info, Path::new("../x"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_rule_built_from_a_path_matches_exactly_that_path() {
    let f = fixture().await;
    let names = [
        "a*b",
        "axb",
        "q?",
        "qz",
        "[x]",
        "x",
        "!bang",
        "#hash",
        "back\\slash",
        "sp ace ",
        "sp ace",
        "dir/in side",
    ];
    for name in names {
        write(f.path(), name, "x\n");
    }
    for name in names {
        let scope = IgnoreScope::Exclude;
        let rule = rule_for_path(Path::new(name), false, &scope).unwrap();
        f.git.add_ignore_rule(&f.info, &scope, &rule).await.unwrap();
        for other in names {
            assert_eq!(
                f.ignored_by_git(other),
                other == name,
                "rule {rule:?} for {name:?} against {other:?}"
            );
        }
        f.git
            .remove_ignore_rule(&f.info, &scope, &rule)
            .await
            .unwrap();
        assert!(!f.ignored_by_git(name), "{name:?} after removal");
    }
    let dir = rule_for_path(
        Path::new("sub/deep"),
        true,
        &IgnoreScope::Directory("sub".into()),
    );
    assert_eq!(dir.unwrap(), "/deep/");
    assert!(
        rule_for_path(
            Path::new("other/x"),
            false,
            &IgnoreScope::Directory("sub".into())
        )
        .is_err()
    );
}
