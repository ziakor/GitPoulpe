mod common;

use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use common::{commit_all, git as run_git, init_repo, write};
use gitpoulpe_core::SessionInfo;
use gitpoulpe_git::{
    Cancel, EDIT_LIMIT, EditError, Git, LineEnding, read_worktree_file, save_worktree_file,
};

async fn session(dir: &Path) -> SessionInfo {
    Git::new()
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .open(dir, &Cancel::new())
        .await
        .unwrap()
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != ".git")
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn saving_rewrites_only_the_announced_file_and_keeps_its_mode_and_directory_clean() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "old\n");
    write(p, "b.txt", "other\n");
    write(p, "run.sh", "#!/bin/sh\n");
    std::fs::set_permissions(p.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    commit_all(p, "init");
    let info = session(p).await;
    let file = read_worktree_file(&info, "a.txt").unwrap();
    let saved = save_worktree_file(&info, &file, "new\nmore\n", false).unwrap();
    assert_eq!(
        std::fs::read_to_string(p.join("a.txt")).unwrap(),
        "new\nmore\n"
    );
    assert_eq!(std::fs::read_to_string(p.join("b.txt")).unwrap(), "other\n");
    assert_eq!(listing(p), ["a.txt", "b.txt", "run.sh"]);
    let status = String::from_utf8(run_git(p, &["status", "--porcelain"])).unwrap();
    assert_eq!(status.trim(), "M a.txt");
    let script = read_worktree_file(&info, "run.sh").unwrap();
    save_worktree_file(&info, &script, "#!/bin/sh\necho hi\n", false).unwrap();
    let mode = std::fs::metadata(p.join("run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
    let again = save_worktree_file(&info, &saved, "third\n", false).unwrap();
    assert_eq!(again.text, "third\n");
}

#[tokio::test]
async fn an_external_change_is_detected_before_overwriting_and_the_choice_is_explicit() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "mine\n");
    commit_all(p, "init");
    let info = session(p).await;
    let file = read_worktree_file(&info, "a.txt").unwrap();
    std::fs::write(p.join("a.txt"), "theirs\n").unwrap();
    assert_eq!(
        save_worktree_file(&info, &file, "mine edited\n", false).unwrap_err(),
        EditError::ExternalChange
    );
    assert_eq!(
        std::fs::read_to_string(p.join("a.txt")).unwrap(),
        "theirs\n"
    );
    assert_eq!(listing(p), ["a.txt"]);
    save_worktree_file(&info, &file, "mine edited\n", true).unwrap();
    assert_eq!(
        std::fs::read_to_string(p.join("a.txt")).unwrap(),
        "mine edited\n"
    );
    let reread = read_worktree_file(&info, "a.txt").unwrap();
    std::fs::write(p.join("a.txt"), "mine edited\n").unwrap();
    assert!(save_worktree_file(&info, &reread, "next\n", false).is_ok());
}

#[tokio::test]
async fn line_endings_and_a_byte_order_mark_survive_a_save() {
    let repo = init_repo();
    let p = repo.path();
    std::fs::write(p.join("crlf.txt"), "a\r\nb\r\n").unwrap();
    std::fs::write(p.join("bom.txt"), b"\xEF\xBB\xBFhello\n").unwrap();
    std::fs::write(p.join("mixed.txt"), "a\r\nb\n").unwrap();
    std::fs::write(p.join("lone.txt"), "a\rb\n").unwrap();
    commit_all(p, "init");
    let info = session(p).await;
    let crlf = read_worktree_file(&info, "crlf.txt").unwrap();
    assert_eq!(
        (crlf.text.as_str(), crlf.line_ending),
        ("a\nb\n", LineEnding::Crlf)
    );
    save_worktree_file(&info, &crlf, "a\nb\nc\n", false).unwrap();
    assert_eq!(
        std::fs::read(p.join("crlf.txt")).unwrap(),
        b"a\r\nb\r\nc\r\n"
    );
    let bom = read_worktree_file(&info, "bom.txt").unwrap();
    assert!(bom.bom && bom.text == "hello\n");
    save_worktree_file(&info, &bom, "bye\n", false).unwrap();
    assert_eq!(
        std::fs::read(p.join("bom.txt")).unwrap(),
        b"\xEF\xBB\xBFbye\n"
    );
    for mixed in ["mixed.txt", "lone.txt"] {
        assert_eq!(
            read_worktree_file(&info, mixed).unwrap_err(),
            EditError::MixedLineEndings
        );
    }
}

#[tokio::test]
async fn binary_unsupported_encodings_and_oversized_files_are_never_rewritten() {
    let repo = init_repo();
    let p = repo.path();
    std::fs::write(p.join("bin.dat"), [0u8, 1, 2, 3]).unwrap();
    std::fs::write(p.join("utf16.txt"), b"\xFF\xFEh\0i\0").unwrap();
    std::fs::write(p.join("latin1.txt"), b"caf\xE9\n").unwrap();
    std::fs::write(p.join("big.txt"), "x".repeat(EDIT_LIMIT as usize + 1)).unwrap();
    commit_all(p, "init");
    let info = session(p).await;
    assert_eq!(
        read_worktree_file(&info, "bin.dat").unwrap_err(),
        EditError::Binary
    );
    assert_eq!(
        read_worktree_file(&info, "utf16.txt").unwrap_err(),
        EditError::UnsupportedEncoding
    );
    assert_eq!(
        read_worktree_file(&info, "latin1.txt").unwrap_err(),
        EditError::UnsupportedEncoding
    );
    assert_eq!(
        read_worktree_file(&info, "big.txt").unwrap_err(),
        EditError::TooLarge
    );
    assert_eq!(std::fs::read(p.join("latin1.txt")).unwrap(), b"caf\xE9\n");
}

#[tokio::test]
async fn paths_outside_the_working_tree_links_and_directories_are_refused() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "a.txt", "x\n");
    std::fs::create_dir(p.join("sub")).unwrap();
    symlink(p.join("a.txt"), p.join("link.txt")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "s\n").unwrap();
    symlink(outside.path(), p.join("escape")).unwrap();
    commit_all(p, "init");
    let info = session(p).await;
    for bad in [
        "",
        "../x",
        "/etc/passwd",
        ".git/config",
        "sub/../../x",
        "escape/secret.txt",
    ] {
        assert_eq!(
            read_worktree_file(&info, bad).unwrap_err(),
            EditError::InvalidPath,
            "{bad}"
        );
    }
    assert_eq!(
        read_worktree_file(&info, "link.txt").unwrap_err(),
        EditError::NotRegular
    );
    assert_eq!(
        read_worktree_file(&info, "sub").unwrap_err(),
        EditError::NotRegular
    );
    assert_eq!(
        read_worktree_file(&info, "missing.txt").unwrap_err(),
        EditError::NotFound
    );
    assert_eq!(
        std::fs::read_to_string(outside.path().join("secret.txt")).unwrap(),
        "s\n"
    );
}

#[tokio::test]
async fn a_read_only_file_or_directory_refuses_the_save_and_leaves_no_trace() {
    let repo = init_repo();
    let p = repo.path();
    write(p, "locked.txt", "keep\n");
    std::fs::create_dir(p.join("dir")).unwrap();
    write(&p.join("dir"), "inner.txt", "keep\n");
    commit_all(p, "init");
    let info = session(p).await;
    let locked = read_worktree_file(&info, "locked.txt").unwrap();
    std::fs::set_permissions(p.join("locked.txt"), std::fs::Permissions::from_mode(0o444)).unwrap();
    assert!(matches!(
        save_worktree_file(&info, &locked, "changed\n", false),
        Err(EditError::PermissionDenied(_))
    ));
    assert_eq!(
        std::fs::read_to_string(p.join("locked.txt")).unwrap(),
        "keep\n"
    );
    let inner = read_worktree_file(&info, "dir/inner.txt").unwrap();
    std::fs::set_permissions(p.join("dir"), std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable_anyway = std::fs::File::create(p.join("dir").join("probe")).is_ok();
    if !writable_anyway {
        assert!(matches!(
            save_worktree_file(&info, &inner, "changed\n", false),
            Err(EditError::PermissionDenied(_))
        ));
        assert_eq!(listing(&p.join("dir")), ["inner.txt"]);
    }
    std::fs::set_permissions(p.join("dir"), std::fs::Permissions::from_mode(0o755)).unwrap();
}
