mod common;

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{git as run_git, git_output, init_repo};
use gitpoulpe_core::{Profile, SessionInfo, SigningPreference};
use gitpoulpe_git::{
    Cancel, Git, IdentityError, SigningCheck, check_signing_with_env, profile_env, signing_config,
};

fn profile(name: &str, email: &str, signing: SigningPreference) -> Profile {
    Profile {
        id: "p1".to_owned(),
        label: "Test".to_owned(),
        user_name: name.to_owned(),
        user_email: email.to_owned(),
        signing,
    }
}

fn isolated_git(dir: &Path, global: Option<&Path>) -> Git {
    let script = dir.join("isolated-git.sh");
    let global = global.map_or_else(|| "/dev/null".to_owned(), |p| p.display().to_string());
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nexport GIT_CONFIG_GLOBAL='{global}' GIT_CONFIG_SYSTEM=/dev/null\nunset EMAIL\nexec git \"$@\"\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    Git::with_program(script)
}

async fn session(git: &Git, path: &Path) -> SessionInfo {
    git.open(path, &Cancel::new()).await.unwrap()
}

#[tokio::test]
async fn a_profile_decides_the_identity_whatever_the_repository_configuration_says() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path(), None);
    let repo = init_repo();
    git_output(repo.path(), &["config", "user.name", "Local Name"]);
    git_output(repo.path(), &["config", "user.email", "local@example.org"]);
    let info = session(&git, repo.path()).await;

    let configured = git
        .effective_identity(&info, None, &Cancel::new())
        .await
        .unwrap();
    assert_eq!(configured.name, "Local Name");
    assert_eq!(configured.email, "local@example.org");

    let work = profile("Ada Lovelace", "ada@work.example", SigningPreference::None);
    let effective = git
        .effective_identity(&info, Some(&work), &Cancel::new())
        .await
        .unwrap();
    assert_eq!(effective.name, "Ada Lovelace");
    assert_eq!(effective.email, "ada@work.example");
    assert_eq!(
        run_git(repo.path(), &["config", "user.name"]),
        b"Local Name\n",
        "reading the identity never rewrites the repository configuration"
    );
}

#[tokio::test]
async fn two_repositories_get_two_identities_and_the_global_configuration_is_untouched() {
    let tools = tempfile::tempdir().unwrap();
    let global = tools.path().join("global.gitconfig");
    std::fs::write(
        &global,
        "[user]\n\tname = Global Person\n\temail = global@example.org\n",
    )
    .unwrap();
    let before = std::fs::read(&global).unwrap();
    let git = isolated_git(tools.path(), Some(&global));
    let (a, b) = (init_repo(), init_repo());
    let (info_a, info_b) = (session(&git, a.path()).await, session(&git, b.path()).await);
    let perso = profile("Ada", "ada@home.example", SigningPreference::None);
    let travail = profile("Ada L.", "ada@work.example", SigningPreference::None);

    let ia = git
        .effective_identity(&info_a, Some(&perso), &Cancel::new())
        .await
        .unwrap();
    let ib = git
        .effective_identity(&info_b, Some(&travail), &Cancel::new())
        .await
        .unwrap();
    assert_eq!(ia.email, "ada@home.example");
    assert_eq!(ib.email, "ada@work.example");
    assert_eq!(
        std::fs::read(&global).unwrap(),
        before,
        "the global file is byte-identical"
    );
    assert!(
        git_output(a.path(), &["config", "--local", "--get", "user.email"])
            .stdout
            .is_empty(),
        "no identity was written to the repository"
    );
}

#[tokio::test]
async fn an_unknown_identity_is_reported_not_invented() {
    let tools = tempfile::tempdir().unwrap();
    let git = isolated_git(tools.path(), None);
    let repo = init_repo();
    git_output(repo.path(), &["config", "user.useConfigOnly", "true"]);
    let info = session(&git, repo.path()).await;
    let error = git
        .effective_identity(&info, None, &Cancel::new())
        .await
        .unwrap_err();
    assert!(matches!(error, IdentityError::Unknown(_)), "{error:?}");
}

#[test]
fn the_profile_is_applied_through_the_environment_and_per_command_signing_settings() {
    let ssh = profile(
        "Ada",
        "ada@example.org",
        SigningPreference::Ssh {
            public_key: "/home/ada/.ssh/id_ed25519.pub".into(),
        },
    );
    let env: Vec<(String, String)> = profile_env(&ssh)
        .into_iter()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.to_string_lossy().into_owned(),
            )
        })
        .collect();
    assert!(env.contains(&("GIT_AUTHOR_NAME".into(), "Ada".into())));
    assert!(env.contains(&("GIT_COMMITTER_EMAIL".into(), "ada@example.org".into())));
    let config = signing_config(&ssh);
    assert!(config.contains(&("gpg.format".into(), "ssh".into())));
    assert!(config.contains(&(
        "user.signingkey".into(),
        "/home/ada/.ssh/id_ed25519.pub".into()
    )));
    let none = profile("Ada", "ada@example.org", SigningPreference::None);
    assert!(
        signing_config(&none).is_empty(),
        "no signing preference changes nothing"
    );
    let gpg = profile(
        "Ada",
        "ada@example.org",
        SigningPreference::Gpg {
            key_id: "ABCD1234".into(),
        },
    );
    assert!(signing_config(&gpg).contains(&("gpg.format".into(), "openpgp".into())));
}

fn ssh_key(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    let private = dir.join(name);
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f"])
        .arg(&private)
        .status()
        .unwrap();
    assert!(status.success());
    let public = dir.join(format!("{name}.pub"));
    (private, public)
}

async fn check(key: &Path) -> SigningCheck {
    let p = profile(
        "Ada",
        "ada@example.org",
        SigningPreference::Ssh {
            public_key: key.to_path_buf(),
        },
    );
    check_signing_with_env(&p, &[]).await
}

#[tokio::test]
async fn an_ssh_reference_must_be_a_readable_public_key_and_a_private_key_is_never_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let (private, public) = ssh_key(dir.path(), "id_test");
    assert_eq!(check(&public).await, SigningCheck::Ready);

    let SigningCheck::Problem(message) = check(&private).await else {
        panic!("a private key must be refused")
    };
    assert!(
        message.contains("clé privée") && message.contains(".pub"),
        "{message}"
    );

    let SigningCheck::Problem(message) = check(&dir.path().join("absent.pub")).await else {
        panic!("a missing key must be reported")
    };
    assert!(
        message.contains("introuvable") && message.contains("chemin"),
        "{message}"
    );

    let garbage = dir.path().join("garbage.pub");
    std::fs::write(&garbage, "hello world, not a key\n").unwrap();
    let SigningCheck::Problem(message) = check(&garbage).await else {
        panic!("garbage must be reported")
    };
    assert!(message.contains("n'est pas une clé publique"), "{message}");

    let SigningCheck::Problem(message) = check(Path::new("relative/id.pub")).await else {
        panic!("a relative path must be reported")
    };
    assert!(message.contains("chemin absolu"), "{message}");
    assert!(
        std::fs::read_to_string(&private)
            .unwrap()
            .starts_with("-----BEGIN"),
        "the private key file was left exactly where it was"
    );
}

#[tokio::test]
async fn a_gpg_reference_is_checked_against_an_isolated_keyring_without_any_secret_handling() {
    let home = tempfile::tempdir().unwrap();
    std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let env: Vec<(OsString, OsString)> = vec![("GNUPGHOME".into(), home.path().into())];
    let status = Command::new("gpg")
        .env("GNUPGHOME", home.path())
        .args([
            "--batch",
            "--passphrase",
            "",
            "--quick-generate-key",
            "Test <test@example.org>",
            "ed25519",
            "sign",
            "never",
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let listing = Command::new("gpg")
        .env("GNUPGHOME", home.path())
        .args(["--batch", "--list-secret-keys", "--with-colons"])
        .output()
        .unwrap();
    let fingerprint = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .find_map(|l| {
            l.strip_prefix("fpr:::::::::")
                .map(|f| f.trim_end_matches(':').to_owned())
        })
        .expect("the key has a fingerprint");

    let with = |id: &str| {
        let p = profile(
            "Ada",
            "ada@example.org",
            SigningPreference::Gpg { key_id: id.into() },
        );
        let env = env.clone();
        async move { check_signing_with_env(&p, &env).await }
    };
    assert_eq!(with(&fingerprint).await, SigningCheck::Ready);
    let SigningCheck::Problem(message) = with("DEADBEEFDEADBEEF").await else {
        panic!("an unknown key must be reported")
    };
    assert!(
        message.contains("Aucune clé secrète") && message.contains("gpg --list-secret-keys"),
        "{message}"
    );
    for bad in ["", "--version", "a b", "x\ny"] {
        let SigningCheck::Problem(message) = with(bad).await else {
            panic!("{bad:?} must be refused")
        };
        assert!(message.contains("invalide"), "{bad:?}: {message}");
    }
    let before = std::fs::read_dir(home.path()).unwrap().count();
    assert!(before > 0);
}
