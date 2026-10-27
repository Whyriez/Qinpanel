//! Database error types.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("Migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("Resource not found: {0}")]
    NotFound(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Validation error: {0}")]
    Validation(String),
}
