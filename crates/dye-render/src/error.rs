use thiserror::Error;

#[derive(Error, Debug)]
#[error("{0}")]
pub struct BackendError(pub String);
