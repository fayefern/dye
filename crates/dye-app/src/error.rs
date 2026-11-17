use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("internal error: {0}")]
    Internal(String),
    #[error("external error: {0}")]
    External(String),
}

#[derive(Error, Debug)]
pub enum AppOptError {
    #[error("file path missing for {0}")]
    Missing(String),
    #[error("file path invalid for {0}")]
    Invalid(String),
    #[error("window size cannot be 0")]
    NullSize,
}
