use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebserverError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Template rendering error: {0}")]
    Template(String),

    #[error("Security violation: {0}")]
    SecurityViolation(String),

    #[error("Nginx configuration syntax check failed: {0}")]
    SyntaxCheckFailed(String),

    #[error("Configuration validation failed (rolled_back: {rolled_back}): {stderr}")]
    ValidationFailed { stderr: String, rolled_back: bool },

    #[error("Rollback failed: {0}")]
    RollbackFailed(String),

    #[error("Service reload failed: {0}")]
    ServiceError(String),
}
