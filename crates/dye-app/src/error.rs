use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("internal error: {0}")]
    Internal(String),
    #[error("external error: {0}")]
    External(String),
}
