use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IgnoreScope {
    Directory(PathBuf),
    Exclude,
    Global,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IgnoreSource {
    Scope(IgnoreScope),
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IgnoreReason {
    pub source: IgnoreSource,
    pub line: u32,
    pub pattern: String,
    pub negated: bool,
    pub tracked: bool,
}

impl IgnoreReason {
    pub fn excludes(&self) -> bool {
        !self.negated && !self.tracked
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IgnoreRuleError {
    NotUtf8,
    OutsideScope,
    Empty,
}

impl std::fmt::Display for IgnoreRuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotUtf8 => "le chemin n'est pas en UTF-8, la règle doit être écrite à la main",
            Self::OutsideScope => "le chemin n'est pas dans le dossier de ce .gitignore",
            Self::Empty => "la règle est vide",
        })
    }
}

impl std::error::Error for IgnoreRuleError {}

pub fn is_plain_relative(path: &Path) -> bool {
    path.components().next().is_some()
        && path.components().all(|c| matches!(c, Component::Normal(_)))
}

pub fn rule_for_path(
    path: &Path,
    is_dir: bool,
    scope: &IgnoreScope,
) -> Result<String, IgnoreRuleError> {
    let relative = match scope {
        IgnoreScope::Directory(base) if !base.as_os_str().is_empty() => path
            .strip_prefix(base)
            .map_err(|_| IgnoreRuleError::OutsideScope)?,
        _ => path,
    };
    if !is_plain_relative(relative) {
        return Err(IgnoreRuleError::Empty);
    }
    let text = relative.to_str().ok_or(IgnoreRuleError::NotUtf8)?;
    let mut rule = String::from("/");
    for ch in text.chars() {
        match ch {
            '*' | '?' | '[' | '\\' => {
                rule.push('\\');
                rule.push(ch);
            }
            _ => rule.push(ch),
        }
    }
    let trimmed = rule.trim_end_matches(' ').len();
    let spaces = rule.len() - trimmed;
    rule.truncate(trimmed);
    rule.push_str(&"\\ ".repeat(spaces));
    if is_dir {
        rule.push('/');
    }
    Ok(rule)
}
