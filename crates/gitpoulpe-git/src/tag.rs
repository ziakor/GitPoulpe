use std::ffi::{OsStr, OsString};

use gitpoulpe_core::{Diagnostic, Profile, SessionInfo, Tag};

use crate::identity::{IdentityError, profile_env};
use crate::process::RunError;
use crate::stage::locked_index;
use crate::{Cancel, Git};

const FIELD: u8 = 0x1f;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagError {
    NoWorktree,
    InvalidName,
    InvalidTarget,
    EmptyMessage,
    AlreadyExists,
    NoSuchTag,
    Changed,
    IdentityUnknown(String),
    Refused(String),
    IndexLocked(String),
    Parse(String),
    GitUnavailable(Diagnostic),
    GitFailed(Diagnostic),
}

impl std::fmt::Display for TagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoWorktree => f.write_str("le dépôt n'a pas de répertoire de travail"),
            Self::InvalidName => f.write_str("nom de tag invalide pour Git"),
            Self::InvalidTarget => f.write_str("le commit visé n'existe pas"),
            Self::EmptyMessage => f.write_str("un tag annoté a besoin d'un message"),
            Self::AlreadyExists => {
                f.write_str("un tag porte déjà ce nom : il n'est jamais remplacé")
            }
            Self::NoSuchTag => f.write_str("ce tag n'existe pas"),
            Self::Changed => {
                f.write_str("le tag a changé depuis l'affichage, rien n'a été supprimé")
            }
            Self::IdentityUnknown(m) => write!(f, "identité inconnue pour signer le tag : {m}"),
            Self::Refused(message) => write!(f, "Git refuse : {message}"),
            Self::IndexLocked(path) => write!(
                f,
                "un autre processus Git travaille sur ce dépôt ({path} existe) ; rien n'a été modifié"
            ),
            Self::Parse(m) => write!(f, "sortie Git inattendue : {m}"),
            Self::GitUnavailable(d) => write!(f, "Git n'a pas pu démarrer : {d}"),
            Self::GitFailed(d) => write!(f, "Git a échoué : {d}"),
        }
    }
}

impl std::error::Error for TagError {}

fn failure(error: RunError) -> TagError {
    match error {
        RunError::Spawn(e) => TagError::GitUnavailable(RunError::spawn_diagnostic(&e)),
        RunError::Io(e) => TagError::GitFailed(RunError::spawn_diagnostic(&e)),
        RunError::Cancelled => TagError::Refused("cancelled".to_owned()),
        RunError::Rejected(m) => TagError::Parse(m),
    }
}

fn valid_oid(id: &str) -> bool {
    (4..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

impl Git {
    async fn tag_run(
        &self,
        info: &SessionInfo,
        args: &[&str],
        env: &[(OsString, OsString)],
        input: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, TagError> {
        let worktree = info.worktree.as_deref().ok_or(TagError::NoWorktree)?;
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let mut out = Vec::new();
        let run = self
            .run_input(worktree, &refs, env, input, &Cancel::new(), |chunk| {
                if out.len() < 4 * 1024 * 1024 {
                    out.extend_from_slice(chunk);
                }
                Ok(())
            })
            .await
            .map_err(failure)?;
        if run.succeeded() {
            return Ok(out);
        }
        let diagnostic = Diagnostic {
            cause: None,
            ..run.diagnostic()
        };
        if let Some(path) = locked_index(&diagnostic) {
            return Err(TagError::IndexLocked(path));
        }
        Err(TagError::Refused(diagnostic.message))
    }

    pub async fn valid_tag_name(&self, info: &SessionInfo, name: &str) -> bool {
        if name.is_empty()
            || name == "@"
            || name == "HEAD"
            || name.starts_with('-')
            || name.contains(['\0', '\n', '\r'])
        {
            return false;
        }
        let full = format!("refs/tags/{name}");
        self.tag_run(info, &["check-ref-format", &full], &[], None)
            .await
            .is_ok()
    }

    pub async fn list_tags(&self, info: &SessionInfo) -> Result<Vec<Tag>, TagError> {
        let out = self
            .tag_run(
                info,
                &[
                    "for-each-ref",
                    "--format=%(refname)%1f%(objectname)%1f%(objecttype)%1f%(*objectname)%1f%(*objecttype)%1f%(taggername)%1f%(taggeremail)%1f%(taggerdate:unix)%1f%(contents:subject)%1f%(contents)%00",
                    "refs/tags",
                ],
                &[],
                None,
            )
            .await?;
        let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        let mut tags = Vec::new();
        for record in out.split(|b| *b == 0) {
            let record = record.strip_prefix(b"\n").unwrap_or(record);
            if record.is_empty() {
                continue;
            }
            let mut f = record.splitn(10, |b| *b == FIELD);
            let (
                Some(name),
                Some(object),
                Some(kind),
                Some(peeled),
                Some(peeled_kind),
                Some(tagger),
                Some(email),
                Some(date),
                Some(subject),
                Some(contents),
            ) = (
                f.next(),
                f.next(),
                f.next(),
                f.next(),
                f.next(),
                f.next(),
                f.next(),
                f.next(),
                f.next(),
                f.next(),
            )
            else {
                return Err(TagError::Parse("missing tag fields".to_owned()));
            };
            let name = text(name);
            let Some(name) = name.strip_prefix("refs/tags/") else {
                return Err(TagError::Parse("unexpected reference".to_owned()));
            };
            let annotated = kind == b"tag";
            let object = text(object);
            let target = if annotated {
                (peeled_kind == b"commit").then(|| text(peeled))
            } else {
                (kind == b"commit").then(|| object.clone())
            };
            let message = annotated.then(|| text(contents).trim_end().to_owned());
            tags.push(Tag {
                name: name.to_owned(),
                object,
                target,
                annotated,
                tagger: (annotated && !tagger.is_empty()).then(|| text(tagger)),
                tagger_email: (annotated && !email.is_empty())
                    .then(|| text(email).trim_matches(['<', '>']).to_owned()),
                timestamp: if annotated {
                    text(date).parse().ok()
                } else {
                    None
                },
                message,
                subject: if annotated {
                    text(subject)
                } else {
                    String::new()
                },
            });
        }
        tags.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tags)
    }

    async fn resolve_target(
        &self,
        info: &SessionInfo,
        target: Option<&str>,
    ) -> Result<String, TagError> {
        let spec = match target {
            None => "HEAD^{commit}".to_owned(),
            Some(oid) if valid_oid(oid) => format!("{oid}^{{commit}}"),
            Some(_) => return Err(TagError::InvalidTarget),
        };
        let out = self
            .tag_run(
                info,
                &["rev-parse", "--verify", "--quiet", &spec],
                &[],
                None,
            )
            .await
            .map_err(|_| TagError::InvalidTarget)?;
        let id = String::from_utf8_lossy(&out).trim().to_owned();
        if valid_oid(&id) {
            Ok(id)
        } else {
            Err(TagError::InvalidTarget)
        }
    }

    pub async fn create_tag(
        &self,
        info: &SessionInfo,
        name: &str,
        target: Option<&str>,
        annotation: Option<&str>,
        profile: Option<&Profile>,
    ) -> Result<Tag, TagError> {
        if !self.valid_tag_name(info, name).await {
            return Err(TagError::InvalidName);
        }
        if self.list_tags(info).await?.iter().any(|t| t.name == name) {
            return Err(TagError::AlreadyExists);
        }
        let commit = self.resolve_target(info, target).await?;
        let env = profile.map(profile_env).unwrap_or_default();
        match annotation {
            None => {
                self.tag_run(info, &["tag", "--", name, &commit], &env, None)
                    .await?;
            }
            Some(message) => {
                let message = message.trim_end();
                if message.trim().is_empty() {
                    return Err(TagError::EmptyMessage);
                }
                match self.effective_identity(info, profile, &Cancel::new()).await {
                    Ok(_) => {}
                    Err(IdentityError::Unknown(m)) => return Err(TagError::IdentityUnknown(m)),
                    Err(IdentityError::Cancelled) => {
                        return Err(TagError::Refused("cancelled".to_owned()));
                    }
                    Err(IdentityError::GitUnavailable(d)) => {
                        return Err(TagError::GitUnavailable(d));
                    }
                    Err(IdentityError::GitFailed(d)) => return Err(TagError::GitFailed(d)),
                }
                let mut env = env;
                env.push((OsString::from("GIT_EDITOR"), OsString::from(":")));
                self.tag_run(
                    info,
                    &[
                        "tag",
                        "-a",
                        "--cleanup=whitespace",
                        "-F",
                        "-",
                        "--",
                        name,
                        &commit,
                    ],
                    &env,
                    Some(format!("{message}\n").into_bytes()),
                )
                .await?;
            }
        }
        self.list_tags(info)
            .await?
            .into_iter()
            .find(|t| t.name == name)
            .ok_or(TagError::NoSuchTag)
    }

    pub async fn delete_tag(
        &self,
        info: &SessionInfo,
        name: &str,
        expected_object: &str,
    ) -> Result<String, TagError> {
        let tag = self
            .list_tags(info)
            .await?
            .into_iter()
            .find(|t| t.name == name)
            .ok_or(TagError::NoSuchTag)?;
        if tag.object != expected_object {
            return Err(TagError::Changed);
        }
        self.tag_run(info, &["tag", "-d", "--", name], &[], None)
            .await?;
        Ok(tag.object)
    }
}
