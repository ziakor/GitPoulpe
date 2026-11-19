mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{commit_all, init_repo, write};
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{Cancel, Git, ToolError, ToolFiles, expand_args, run_tool, split_args};

const SCRIPT: &str = "#!/bin/sh\nlog=\"$1\"; code=\"$2\"; shift 2\n: > \"$log\"\nfor a in \"$@\"; do\n  printf 'ARG<%s>\\n' \"$a\" >> \"$log\"\n  if [ -f \"$a\" ]; then printf 'CONTENT<%s>\\n' \"$(cat \"$a\")\" >> \"$log\"; fi\ndone\nprintf 'CWD<%s>\\n' \"$(pwd -P)\" >> \"$log\"\necho 'boom detail' >&2\nexit \"$code\"\n";

fn tool(dir: &Path) -> PathBuf {
    let path = dir.join("fake tool.sh");
    std::fs::write(&path, SCRIPT).unwrap();
    path
}

async fn session(dir: &Path) -> (Git, SessionInfo) {
    let git = Git::new().with_env("GIT_CONFIG_GLOBAL", "/dev/null");
    let info = git.open(dir, &Cancel::new()).await.unwrap();
    (git, info)
}

#[test]
fn arguments_are_split_like_a_shell_would_without_running_one() {
    assert_eq!(split_args("a b  c").unwrap(), ["a", "b", "c"]);
    assert_eq!(
        split_args("--wait \"my file.txt\" 'x y'").unwrap(),
        ["--wait", "my file.txt", "x y"]
    );
    assert_eq!(
        split_args("\"a \\\"quoted\\\" b\"").unwrap(),
        ["a \"quoted\" b"]
    );
    assert_eq!(
        split_args("pre\"mid dle\"post").unwrap(),
        ["premid dlepost"]
    );
    assert_eq!(split_args("''").unwrap(), [""]);
    assert_eq!(split_args("   ").unwrap(), Vec::<String>::new());
    assert_eq!(
        split_args("$HOME `id` ; rm").unwrap(),
        ["$HOME", "`id`", ";", "rm"]
    );
    assert!(matches!(
        split_args("'open"),
        Err(ToolError::BadArguments(_))
    ));
    assert!(matches!(
        split_args("\"open"),
        Err(ToolError::BadArguments(_))
    ));
}

#[test]
fn placeholders_are_replaced_inside_one_argument_and_unknown_ones_are_refused() {
    let vars = [("file", "/r/we ird 'q\" $(x);*é.txt"), ("repo", "/r")];
    assert_eq!(
        expand_args("--goto {file}:1 -C {repo} {file}", &vars).unwrap(),
        [
            "--goto",
            "/r/we ird 'q\" $(x);*é.txt:1",
            "-C",
            "/r",
            "/r/we ird 'q\" $(x);*é.txt"
        ]
    );
    assert_eq!(
        expand_args("{ not closed", &vars).unwrap(),
        ["{", "not", "closed"]
    );
    assert_eq!(
        expand_args("{left}", &vars).unwrap_err(),
        ToolError::UnknownPlaceholder("left".to_owned())
    );
    assert!(expand_args("{file", &vars).is_ok());
    assert_eq!(
        expand_args("{file}{file}", &[("file", "$x")]).unwrap(),
        ["$x$x"]
    );
}

#[tokio::test]
async fn the_tool_receives_the_exact_paths_and_versions_even_with_special_characters() {
    let repo = init_repo();
    let p = repo.path();
    let name = "we ird 'q\" $(x);*é.txt";
    write(p, name, "version one\n");
    commit_all(p, "one");
    write(p, name, "version two\n");
    commit_all(p, "two");
    let (git, info) = session(p).await;
    let files = ToolFiles::new().unwrap();
    let left = git
        .export_version(&info, &format!("HEAD^:{name}"), &files, "left", name)
        .await
        .unwrap();
    let right = git
        .export_version(&info, &format!("HEAD:{name}"), &files, "right", name)
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&left).unwrap(), "version one\n");
    assert_eq!(std::fs::read_to_string(&right).unwrap(), "version two\n");
    assert_ne!(left, right);
    let scratch = tempfile::tempdir().unwrap();
    let program = tool(scratch.path());
    let log = scratch.path().join("log.txt");
    let vars = [
        ("left", left.to_str().unwrap()),
        ("right", right.to_str().unwrap()),
    ];
    let args = expand_args(&format!("'{}' 0 {{left}} {{right}}", log.display()), &vars).unwrap();
    let mut full = vec![program.display().to_string()];
    full.extend(args);
    run_tool("sh", &full, p).await.unwrap();
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        recorded,
        format!(
            "ARG<{}>\nCONTENT<version one>\nARG<{}>\nCONTENT<version two>\nCWD<{}>\n",
            left.display(),
            right.display(),
            p.canonicalize().unwrap().display()
        )
    );
}

#[tokio::test]
async fn a_failing_missing_or_non_executable_tool_gives_a_recoverable_error() {
    let scratch = tempfile::tempdir().unwrap();
    let program = tool(scratch.path());
    let log = scratch.path().join("log.txt");
    let failed = run_tool(
        "sh",
        &[
            program.display().to_string(),
            log.display().to_string(),
            "3".to_owned(),
        ],
        scratch.path(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        failed,
        ToolError::Failed {
            code: Some(3),
            detail: "boom detail".to_owned()
        }
    );
    assert!(failed.to_string().contains("code 3"));
    assert_eq!(
        run_tool("definitely-not-installed-xyz", &[], scratch.path())
            .await
            .unwrap_err(),
        ToolError::NotFound("definitely-not-installed-xyz".to_owned())
    );
    let plain = scratch.path().join("plain.txt");
    std::fs::write(&plain, "x").unwrap();
    assert!(matches!(
        run_tool(plain.to_str().unwrap(), &[], scratch.path()).await,
        Err(ToolError::PermissionDenied(_))
    ));
    assert_eq!(
        run_tool("  ", &[], scratch.path()).await.unwrap_err(),
        ToolError::Empty
    );
}

#[tokio::test]
async fn temporary_versions_are_private_cleaned_up_and_empty_when_the_side_does_not_exist() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "only.txt", "content\n");
    commit_all(p, "root");
    let (git, info) = session(p).await;
    let root_dir;
    {
        let files = ToolFiles::new().unwrap();
        root_dir = files.root().to_path_buf();
        let mode = std::fs::metadata(&root_dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "the temporary folder is private");
        let missing = git
            .export_version(&info, "HEAD^:only.txt", &files, "left", "only.txt")
            .await
            .unwrap();
        assert_eq!(std::fs::read(&missing).unwrap(), b"");
        let nested = git
            .export_version(
                &info,
                "HEAD:only.txt",
                &files,
                "right",
                "../../escape/only.txt",
            )
            .await
            .unwrap();
        assert!(nested.starts_with(files.root()));
        assert!(root_dir.exists());
    }
    assert!(!root_dir.exists());
}
