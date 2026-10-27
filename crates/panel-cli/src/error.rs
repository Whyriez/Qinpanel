//! Error types for `panelctl`.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("Database operation failed: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("Database error: {0}")]
    Db(#[from] panel_db::error::DbError),

    #[error("Core error: {0}")]
    Core(#[from] panel_core::CoreError),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}
