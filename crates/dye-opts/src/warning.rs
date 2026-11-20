use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug)]
pub struct OptWarning {
    pub path: PathBuf,
    pub kind: WarningKind,
}

#[derive(Error, Debug)]
pub enum WarningKind {
    #[error("could not read file: {0}")]
    Io(String),
    #[error("syntax error{}: {message}", line.map(|l| format!(" at line {l}")).unwrap_or_default())]
    Syntax {
        line: Option<usize>,
        message: String,
    },
    #[error("section `{section}` is invalid and was reset to defaults: {message}")]
    BadSection { section: String, message: String },
    #[error("unknown key `{0}` ignored")]
    UnknownKey(String),
    #[error("file version {found} is newer than supported version {supported}")]
    NewerVersion { found: u32, supported: u32 },
    #[error("`{field}` {reason}")]
    Invalid { field: String, reason: String },
}

#[derive(Default)]
pub struct Issues(pub Vec<(String, String)>);

impl Issues {
    pub fn invalid(&mut self, field: impl Into<String>, reason: impl Into<String>) {
        self.0.push((field.into(), reason.into()));
    }
}
