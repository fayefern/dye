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
