use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnalyticsError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("System error: {0}")]
    System(String),

    #[error("Internal error: {0}")]
    Internal(String),
}
