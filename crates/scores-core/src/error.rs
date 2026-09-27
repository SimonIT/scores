use thiserror::Error;

/// Errors that can occur while reading or writing a [`crate::Score`] through a
/// [`crate::Format`] implementation.
#[derive(Debug, Error)]
pub enum ScoresError {
    #[error("failed to parse input: {0}")]
    Parse(String),

    #[error("failed to write output: {0}")]
    Write(String),

    #[error("feature not supported by this format: {0}")]
    Unsupported(String),

    #[error("unknown format: {0}")]
    UnknownFormat(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ScoresError>;
