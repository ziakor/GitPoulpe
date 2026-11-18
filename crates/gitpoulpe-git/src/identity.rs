use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use gitpoulpe_core::{Diagnostic, Profile, SessionInfo, SigningPreference};

use crate::diagnostics::filter_diagnostic;
use crate::process::RunError;
use crate::{Cancel, Git};

const SIGNING_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const SSH_KEY_PREFIXES: [&str; 6] = [
    "ssh-ed25519 ",
    "ssh-rsa ",
    "ecdsa-sha2-",
    "sk-ssh-ed25519@openssh.com ",
    "sk-ecdsa-sha2-",
    "ssh-dss ",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityError {
    Unknown(String),
    Cancelled,
    GitUnavailable(Diagnostic),
    GitFailed(Diagnostic),
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(message) => f.write_str(message),
            Self::Cancelled => f.write_str("the request was cancelled"),
            Self::GitUnavailable(d) => write!(f, "Git could not be started: {d}"),
            Self::GitFailed(d) => write!(f, "Git failed: {d}"),
        }
    }
}

impl std::error::Error for IdentityError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SigningCheck {
    NotRequested,
    Ready,
    Problem(String),
}

pub fn profile_env(profile: &Profile) -> Vec<(OsString, OsString)> {
    [
        ("GIT_AUTHOR_NAME", &profile.user_name),
        ("GIT_AUTHOR_EMAIL", &profile.user_email),
        ("GIT_COMMITTER_NAME", &profile.user_name),
        ("GIT_COMMITTER_EMAIL", &profile.user_email),
    ]
    .into_iter()
    .map(|(name, value)| (OsString::from(name), OsString::from(value)))
    .collect()
}

pub fn signing_config(profile: &Profile) -> Vec<(String, String)> {
    match &profile.signing {
        SigningPreference::None => Vec::new(),
        SigningPreference::Ssh { public_key } => vec![
            ("gpg.format".to_owned(), "ssh".to_owned()),
            (
                "user.signingkey".to_owned(),
                public_key.to_string_lossy().into_owned(),
            ),
            ("commit.gpgsign".to_owned(), "true".to_owned()),
        ],
        SigningPreference::Gpg { key_id } => vec![
            ("gpg.format".to_owned(), "openpgp".to_owned()),
            ("user.signingkey".to_owned(), key_id.clone()),
            ("commit.gpgsign".to_owned(), "true".to_owned()),
        ],
    }
}

fn parse_ident(output: &[u8]) -> Option<Identity> {
    let text = String::from_utf8_lossy(output);
    let line = text.lines().next()?;
    let open = line.rfind('<')?;
    let close = line.rfind('>')?;
    (open < close).then(|| Identity {
        name: line[..open].trim().to_owned(),
        email: line[open + 1..close].to_owned(),
    })
}

impl Git {
    pub async fn effective_identity(
        &self,
        info: &SessionInfo,
        profile: Option<&Profile>,
        cancel: &Cancel,
    ) -> Result<Identity, IdentityError> {
        let dir = info.worktree.as_deref().unwrap_or(&info.git_dir);
        let args: Vec<OsString> = ["var", "GIT_AUTHOR_IDENT"].map(OsString::from).to_vec();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let env = profile.map(profile_env).unwrap_or_default();
        let mut out = Vec::new();
        let run = self
            .run_env(dir, &refs, &env, cancel, |chunk| {
                out.extend_from_slice(chunk);
                Ok(())
            })
            .await
            .map_err(|e| match e {
                RunError::Cancelled => IdentityError::Cancelled,
                RunError::Spawn(e) => IdentityError::GitUnavailable(RunError::spawn_diagnostic(&e)),
                RunError::Io(e) => IdentityError::GitFailed(RunError::spawn_diagnostic(&e)),
                RunError::Rejected(message) => IdentityError::GitFailed(Diagnostic {
                    exit_code: None,
                    signal: None,
                    message,
                    cause: None,
                }),
            })?;
        if !run.succeeded() {
            let message = filter_diagnostic(&run.stderr);
            return Err(IdentityError::Unknown(message));
        }
        parse_ident(&out).ok_or_else(|| {
            IdentityError::Unknown("Git a renvoyé une identité illisible.".to_owned())
        })
    }
}

pub async fn check_signing(profile: &Profile) -> SigningCheck {
    check_signing_with_env(profile, &[]).await
}

pub async fn check_signing_with_env(
    profile: &Profile,
    env: &[(OsString, OsString)],
) -> SigningCheck {
    match &profile.signing {
        SigningPreference::None => SigningCheck::NotRequested,
        SigningPreference::Ssh { public_key } => check_ssh_key(public_key),
        SigningPreference::Gpg { key_id } => check_gpg_key(key_id, env).await,
    }
}

fn check_ssh_key(path: &Path) -> SigningCheck {
    let show = path.display();
    if !path.is_absolute() {
        return SigningCheck::Problem(format!(
            "La clé « {show} » doit être indiquée par un chemin absolu vers la clé publique (fichier .pub)."
        ));
    }
    let mut head = [0u8; 64];
    let read = std::fs::File::open(path).and_then(|mut file| file.read(&mut head));
    let Ok(count) = read else {
        return SigningCheck::Problem(format!(
            "La clé « {show} » est introuvable ou illisible. Corrige le chemin du fichier .pub dans le profil."
        ));
    };
    let head = String::from_utf8_lossy(&head[..count]);
    if head.starts_with("-----BEGIN") {
        return SigningCheck::Problem(format!(
            "« {show} » est une clé privée. Indique la clé publique correspondante (fichier .pub) : GitPoulpe ne lit ni ne copie jamais les clés privées."
        ));
    }
    if !SSH_KEY_PREFIXES
        .iter()
        .any(|prefix| head.starts_with(prefix))
    {
        return SigningCheck::Problem(format!(
            "« {show} » n'est pas une clé publique SSH. Indique le fichier .pub (par exemple id_ed25519.pub)."
        ));
    }
    SigningCheck::Ready
}

async fn check_gpg_key(key_id: &str, env: &[(OsString, OsString)]) -> SigningCheck {
    let id = key_id.trim();
    if id.is_empty()
        || id.starts_with('-')
        || id.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return SigningCheck::Problem(
            "L'identifiant de clé GPG est invalide : utilise l'empreinte ou l'identifiant long (gpg --list-secret-keys --keyid-format long)."
                .to_owned(),
        );
    }
    let mut command = tokio::process::Command::new("gpg");
    command
        .args(["--batch", "--list-secret-keys", "--with-colons", "--"])
        .arg(id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command.envs(env.iter().map(|(k, v)| (k, v)));
    let status = tokio::time::timeout(SIGNING_CHECK_TIMEOUT, command.status()).await;
    match status {
        Err(_) => SigningCheck::Problem(
            "gpg ne répond pas : vérifie le trousseau puis réessaie.".to_owned(),
        ),
        Ok(Err(_)) => SigningCheck::Problem(
            "gpg est introuvable : installe GnuPG ou choisis une autre signature dans le profil."
                .to_owned(),
        ),
        Ok(Ok(status)) if status.success() => SigningCheck::Ready,
        Ok(Ok(_)) => SigningCheck::Problem(format!(
            "Aucune clé secrète GPG ne correspond à « {id} ». Vérifie l'identifiant avec « gpg --list-secret-keys --keyid-format long »."
        )),
    }
}
