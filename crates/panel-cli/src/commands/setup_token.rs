//! Compatibility helpers for retired first-run tokens. Tokens can no longer authorize setup.
use crate::error::CliError;
use panel_db::pool::DbPool;
pub const SETTING_SETUP_TOKEN: &str = "setup.first_run_token";
pub const SETTING_SETUP_TOKEN_CREATED_AT: &str = "setup.first_run_token_created_at";
#[derive(Debug, PartialEq, Eq)]
pub enum SetupStatus {
    Completed,
    /// Retained for source compatibility; never returned.
    Pending {
        token: String,
        created_at: Option<i64>,
    },
    Uninitialized,
}
pub async fn generate_setup_token(_pool: &DbPool) -> Result<String, CliError> {
    Err(CliError::Validation(
        "Setup token sudah dihentikan; gunakan panelctl admin bootstrap.".into(),
    ))
}
pub async fn get_setup_status(pool: &DbPool) -> Result<SetupStatus, CliError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role='admin'")
        .fetch_one(pool)
        .await?;
    Ok(if count > 0 {
        SetupStatus::Completed
    } else {
        SetupStatus::Uninitialized
    })
}
pub async fn verify_setup_token(_pool: &DbPool, _candidate: &str) -> Result<bool, CliError> {
    Ok(false)
}
