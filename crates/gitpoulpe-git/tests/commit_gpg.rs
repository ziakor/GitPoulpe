mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{commit_all, git as run_git, git_output, init_repo, write};
use gitpoulpe_core::{
    CommitFailureKind, CommitOutcome, CommitRequest, Generation, Profile, SessionInfo,
    SignatureKind, SigningPreference, StatusSnapshot,
};
use gitpoulpe_git::{Cancel, Git};

const PASSPHRASE: &str = "s3cret-passphrase-4-tests";

struct Keyring {
    home: tempfile::TempDir,
}

impl Keyring {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        Self { home }
    }

    fn gpg(&self, args: &[&str]) -> std::process::Output {
        Command::new("gpg")
            .env("GNUPGHOME", self.home.path())
            .args(args)
            .output()
            .unwrap()
    }

    fn generate(&self, passphrase: &str) -> String {
        let out = self.gpg(&[
            "--batch",
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            passphrase,
            "--quick-generate-key",
            "Ada Lovelace <ada@example.org>",
            "ed25519",
            "sign",
            "never",
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let listing = self.gpg(&["--batch", "--list-secret-keys", "--with-colons"]);
        String::from_utf8_lossy(&listing.stdout)
            .lines()
            .find_map(|l| {
                l.strip_prefix("fpr:::::::::")
                    .map(|f| f.trim_end_matches(':').to_owned())
            })
            .expect("the key has a fingerprint")
    }

    fn pinentry(&self, body: &str) {
        let script = self.home.path().join("pinentry-test.sh");
        std::fs::write(&script, format!("#!/bin/sh\necho 'OK ready'\nwhile read -r line; do\n  case \"$line\" in\n{body}\n    BYE*) echo 'OK bye'; exit 0 ;;\n    *) echo OK ;;\n  esac\ndone\n")).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            self.home.path().join("gpg-agent.conf"),
            format!(
                "pinentry-program {}\nallow-loopback-pinentry\n",
                script.display()
            ),
        )
        .unwrap();
        let _ = Command::new("gpgconf")
            .env("GNUPGHOME", self.home.path())
            .args(["--kill", "gpg-agent"])
            .output();
    }

    fn git(&self, dir: &Path) -> Git {
        let script = dir.join("isolated-git.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nexport GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null GNUPGHOME='{}'\nunset EMAIL GPG_TTY\nexec git \"$@\"\n",
                self.home.path().display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        Git::with_program(script).with_env("GNUPGHOME", self.home.path())
    }
}

impl Drop for Keyring {
    fn drop(&mut self) {
        let _ = Command::new("gpgconf")
            .env("GNUPGHOME", self.home.path())
            .args(["--kill", "gpg-agent"])
            .output();
    }
}

fn profile(key_id: &str) -> Profile {
    Profile {
        id: "p1".into(),
        label: "GPG".into(),
        user_name: "Ada Lovelace".into(),
        user_email: "ada@example.org".into(),
        signing: SigningPreference::Gpg {
            key_id: key_id.to_owned(),
        },
    }
}

fn repo_with_staged_change() -> tempfile::TempDir {
    let repo = init_repo();
    write(repo.path(), "a.txt", "one\n");
    commit_all(repo.path(), "base");
    write(repo.path(), "a.txt", "one\ntwo\n");
    run_git(repo.path(), &["add", "a.txt"]);
    repo
}

async fn open(git: &Git, dir: &Path) -> (SessionInfo, StatusSnapshot) {
    let info = git.open(dir, &Cancel::new()).await.unwrap();
    let snapshot = git
        .read_status(&info, Generation::default().next(), &Cancel::new())
        .await
        .unwrap();
    (info, snapshot)
}

fn request(snapshot: &StatusSnapshot, profile: Profile) -> CommitRequest {
    CommitRequest {
        message: "Signed with GPG".to_owned(),
        staged: snapshot
            .entries
            .iter()
            .filter(|e| e.is_staged())
            .map(|e| {
                (
                    e.path.clone(),
                    snapshot.index_ids.get(&e.path).cloned().unwrap_or_default(),
                )
            })
            .collect(),
        profile: Some(profile),
    }
}

fn head(dir: &Path) -> String {
    String::from_utf8(run_git(dir, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned()
}

fn cancelling_pinentry() -> &'static str {
    "    GETPIN*) echo 'ERR 83886179 Operation cancelled' ;;"
}

fn typing_pinentry() -> String {
    format!("    GETPIN*) echo 'D {PASSPHRASE}'; echo OK ;;")
}

#[tokio::test]
async fn a_gpg_signed_commit_is_verifiable_and_the_passphrase_goes_through_the_pinentry_only() {
    let keyring = Keyring::new();
    let fingerprint = keyring.generate(PASSPHRASE);
    keyring.pinentry(&typing_pinentry());
    let tools = tempfile::tempdir().unwrap();
    let git = keyring.git(tools.path());
    let repo = repo_with_staged_change();
    let dir = repo.path();
    let (info, snapshot) = open(&git, dir).await;
    let profile = profile(&fingerprint);
    let stored = serde_like(&profile);

    let outcome = git
        .commit(&info, &request(&snapshot, profile), &Cancel::new())
        .await
        .unwrap();
    let CommitOutcome::Created { signature, .. } = &outcome else {
        panic!("{outcome:?}")
    };
    let signature = signature.expect("the commit is signed");
    assert_eq!(signature.kind, SignatureKind::Gpg);
    assert!(
        signature.verified,
        "GitPoulpe verified the signature with Git"
    );

    let mut verify = Command::new("git");
    verify
        .current_dir(dir)
        .env("GNUPGHOME", keyring.home.path())
        .args(["verify-commit", "HEAD"]);
    let verified = verify.output().unwrap();
    assert!(
        verified.status.success(),
        "Git verifies the signature: {}",
        String::from_utf8_lossy(&verified.stderr)
    );
    assert!(
        String::from_utf8_lossy(&run_git(dir, &["cat-file", "commit", "HEAD"]))
            .contains("BEGIN PGP SIGNATURE")
    );

    assert!(
        !format!("{outcome:?}").contains(PASSPHRASE),
        "no passphrase in the result"
    );
    assert!(
        !stored.contains(PASSPHRASE),
        "no passphrase in the profile that would be stored"
    );
    let in_repo = Command::new("grep")
        .args(["-rl", PASSPHRASE])
        .arg(dir.join(".git"))
        .output()
        .unwrap();
    assert!(in_repo.stdout.is_empty(), "no passphrase in the repository");
}

fn serde_like(profile: &Profile) -> String {
    format!("{profile:?}")
}

#[tokio::test]
async fn cancelling_the_unlock_dialog_never_looks_like_a_success() {
    let keyring = Keyring::new();
    let fingerprint = keyring.generate(PASSPHRASE);
    keyring.pinentry(cancelling_pinentry());
    let tools = tempfile::tempdir().unwrap();
    let git = keyring.git(tools.path());
    let repo = repo_with_staged_change();
    let dir = repo.path();
    let (info, snapshot) = open(&git, dir).await;
    let head_before = head(dir);
    let index_before = run_git(dir, &["ls-files", "--stage", "-z"]);

    let outcome = git
        .commit(
            &info,
            &request(&snapshot, profile(&fingerprint)),
            &Cancel::new(),
        )
        .await
        .unwrap();
    let CommitOutcome::Failed { kind, output } = outcome else {
        panic!("a cancelled unlock must not create a commit: {outcome:?}")
    };
    assert_eq!(kind, CommitFailureKind::SigningUnavailable);
    assert!(!output.contains(PASSPHRASE));
    assert_eq!(head(dir), head_before, "no commit was created");
    assert_eq!(
        run_git(dir, &["ls-files", "--stage", "-z"]),
        index_before,
        "the work is still staged"
    );

    keyring.pinentry(&typing_pinentry());
    let (info, snapshot) = open(&git, dir).await;
    let retry = git
        .commit(
            &info,
            &request(&snapshot, profile(&fingerprint)),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert!(
        matches!(retry, CommitOutcome::Created { .. }),
        "after unlocking, the same commit works: {retry:?}"
    );
}

#[tokio::test]
async fn an_unavailable_key_is_reported_before_anything_is_written() {
    let keyring = Keyring::new();
    let _ = keyring.generate(PASSPHRASE);
    keyring.pinentry(&typing_pinentry());
    let tools = tempfile::tempdir().unwrap();
    let git = keyring.git(tools.path());
    let repo = repo_with_staged_change();
    let dir = repo.path();
    let (info, snapshot) = open(&git, dir).await;
    let head_before = head(dir);
    let outcome = git
        .commit(
            &info,
            &request(&snapshot, profile("DEADBEEFDEADBEEF")),
            &Cancel::new(),
        )
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        CommitOutcome::Failed {
            kind: CommitFailureKind::SigningUnavailable,
            ..
        }
    ));
    assert_eq!(head(dir), head_before);
    let _ = (git_output, PathBuf::new());
}
