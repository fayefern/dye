use std::fmt::Display;

use thiserror::Error;

#[derive(Error, Clone, Debug, PartialEq, Eq)]
pub enum PathError {
    #[error("empty path")]
    Empty,
    #[error("absolute paths are not allowed")]
    Absolute,
    #[error("backslashes are not allowed. use `/`")]
    Backslash,
    #[error("`.`, `..` and empty path segments are not allowed")]
    BadSegment,
    #[error("uppercase letters are not allowed")]
    Uppercase,
    #[error("invalid character {0:?}")]
    BadChar(char),
    #[error("symlinks are not allowed")]
    Symlink,
    #[error("file name is not valid UTF-8")]
    NotUtf8,
    #[error("duplicate entry")]
    Duplicate,
}

#[derive(Error, Clone, Debug)]
pub enum AssetError {
    #[error("could not read `{path}`: {message}")]
    Io { path: String, message: String },
    #[error("{0}")]
    Parse(String),
    #[error("the parser panicked")]
    Panic,
}

#[derive(Error, Clone, Debug)]
pub enum Problem {
    #[error("no pack provides it, using nil")]
    Missing,
    #[error("failed to load, using nil: {0}")]
    Failed(AssetError),
    #[error("invalid id ({0}), using nil")]
    BadId(String),
    #[error("{0}")]
    Warning(String),
}

#[derive(Clone, Debug)]
pub struct AssetDiag {
    pub kind: &'static str,
    pub subject: String,
    pub problem: Problem,
}

impl Display for AssetDiag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} `{}`: {}", self.kind, self.subject, self.problem)
    }
}

#[derive(Error, Clone, Debug, PartialEq, Eq)]
pub enum IdError {
    #[error("`{0}` is not a valid namespace (a..=z, 0..=9, `_`, starting with a letter)")]
    Namespace(String),
    #[error("`{0}` is not a valid identity (a..=z, 0..=9, `_`, with `/` between folders)")]
    Identity(String),
    #[error("`{0}` is not formatted like an asset_id (`namespace:identity`)")]
    Format(String),
}
