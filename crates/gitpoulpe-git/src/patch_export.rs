use std::io::Write;
use std::path::Path;

use gitpoulpe_core::SessionInfo;

use crate::Git;
use crate::patch::PATCH_LIMIT;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportSource {
    WorkingTree,
    Index,
    Commits(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    Nothing,
    TooLarge,
    BadRevision(String),
    Git(String),
    Destination(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nothing => f.write_str("rien à exporter pour cette source"),
            Self::TooLarge => f.write_str("le patch dépasse 8 Mio : export refusé"),
            Self::BadRevision(r) => write!(f, "révision invalide : {r}"),
            Self::Git(why) => write!(f, "git a refusé l'export : {why}"),
            Self::Destination(why) => write!(f, "destination inutilisable : {why}"),
        }
    }
}

impl std::error::Error for ExportError {}

fn valid_revision(spec: &str) -> bool {
    let spec = spec.trim();
    !spec.is_empty()
        && !spec.starts_with('-')
        && !spec.chars().any(|c| c.is_control() || c.is_whitespace())
}

impl Git {
    pub async fn export_patch(
        &self,
        info: &SessionInfo,
        source: &ExportSource,
    ) -> Result<Vec<u8>, ExportError> {
        let revision;
        let args: Vec<&str> = match source {
            ExportSource::WorkingTree => vec!["diff", "--binary", "--no-ext-diff", "--no-color"],
            ExportSource::Index => vec![
                "diff",
                "--cached",
                "--binary",
                "--no-ext-diff",
                "--no-color",
            ],
            ExportSource::Commits(spec) => {
                if !valid_revision(spec) {
                    return Err(ExportError::BadRevision(spec.trim().to_owned()));
                }
                revision = spec.trim().to_owned();
                let mut args = vec!["format-patch", "--stdout", "--binary"];
                if !revision.contains("..") {
                    args.push("-1");
                }
                args.push(&revision);
                args.push("--");
                args
            }
        };
        let (run, out) = self
            .collect_in(info, &args)
            .await
            .map_err(|e| ExportError::Git(e.to_string()))?;
        if !run.succeeded() {
            return Err(ExportError::Git(run.diagnostic().message));
        }
        if out.iter().all(u8::is_ascii_whitespace) {
            return Err(ExportError::Nothing);
        }
        if out.len() > PATCH_LIMIT {
            return Err(ExportError::TooLarge);
        }
        Ok(out)
    }
}

pub fn write_export(destination: &Path, bytes: &[u8], overwrite: bool) -> Result<(), ExportError> {
    let fail = |why: String| ExportError::Destination(why);
    if !destination.is_absolute() {
        return Err(fail("indiquez un chemin absolu".to_owned()));
    }
    let Some(parent) = destination.parent() else {
        return Err(fail("chemin sans dossier parent".to_owned()));
    };
    if !parent.is_dir() {
        return Err(fail(format!(
            "le dossier {} n'existe pas",
            parent.display()
        )));
    }
    match std::fs::symlink_metadata(destination) {
        Ok(meta) if meta.is_dir() => return Err(fail("la destination est un dossier".to_owned())),
        Ok(_) if !overwrite => {
            return Err(fail(
                "le fichier existe déjà : confirmez le remplacement".to_owned(),
            ));
        }
        Ok(_) | Err(_) => {}
    }
    let mut temp = tempfile::Builder::new()
        .prefix(".gitpoulpe-export-")
        .tempfile_in(parent)
        .map_err(|e| fail(e.to_string()))?;
    temp.write_all(bytes)
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|e| fail(format!("écriture interrompue : {e}")))?;
    if overwrite {
        temp.persist(destination)
            .map_err(|e| fail(e.error.to_string()))?;
    } else {
        temp.persist_noclobber(destination)
            .map_err(|e| fail(e.error.to_string()))?;
    }
    Ok(())
}
