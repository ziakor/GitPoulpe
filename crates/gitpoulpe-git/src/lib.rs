mod cancel;
mod diagnostics;
mod open;
mod parse;
mod process;
mod status;

pub use cancel::Cancel;
pub use diagnostics::filter_diagnostic;
pub use parse::{ParseError, StatusParser};

use std::ffi::OsString;

#[derive(Clone, Debug)]
pub struct Git {
    program: OsString,
}

impl Git {
    pub fn new() -> Self {
        Self {
            program: OsString::from("git"),
        }
    }

    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }
}

impl Default for Git {
    fn default() -> Self {
        Self::new()
    }
}
