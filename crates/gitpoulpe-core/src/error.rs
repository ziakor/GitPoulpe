use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub message: String,
    pub cause: Option<String>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.exit_code, self.signal) {
            (Some(code), _) => write!(f, "git exited with code {code}")?,
            (None, Some(signal)) => write!(f, "git was killed by signal {signal}")?,
            (None, None) => write!(f, "git did not run")?,
        }
        if !self.message.is_empty() {
            write!(f, ": {}", self.message)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    NotFound,
    NotADirectory,
    PermissionDenied,
    NotARepository,
    GitUnavailable(Diagnostic),
    GitFailed(Diagnostic),
    Cancelled,
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("the path does not exist"),
            Self::NotADirectory => f.write_str("the path is not a directory"),
            Self::PermissionDenied => f.write_str("access to the path was denied"),
            Self::NotARepository => f.write_str("the path is not a Git repository"),
            Self::GitUnavailable(d) => write!(f, "Git could not be started: {d}"),
            Self::GitFailed(d) => write!(f, "Git failed: {d}"),
            Self::Cancelled => f.write_str("the request was cancelled"),
        }
    }
}

impl std::error::Error for OpenError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    NoWorktree,
    Cancelled,
    GitUnavailable(Diagnostic),
    GitFailed(Diagnostic),
    Parse(String),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoWorktree => f.write_str("the repository has no working directory"),
            Self::Cancelled => f.write_str("the request was cancelled"),
            Self::GitUnavailable(d) => write!(f, "Git could not be started: {d}"),
            Self::GitFailed(d) => write!(f, "Git failed: {d}"),
            Self::Parse(m) => write!(f, "unexpected Git output: {m}"),
        }
    }
}

impl std::error::Error for ReadError {}
