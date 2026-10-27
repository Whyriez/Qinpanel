//! Admin management commands for `panelctl`.

use crate::error::CliError;
use panel_core::auth::hash_password;
use panel_db::pool::DbPool;
use panel_db::repositories::SqliteUserRepository;

/// Plaintext credentials are returned only to the invoking local operator, never stored.
#[derive(serde::Serialize)]
pub struct BootstrapCredentials {
    pub username: String,
    pub password: String,
}

pub fn generate_admin_password() -> String {
    format!("Qp!{}", uuid::Uuid::new_v4().simple())
}

/// A write lock makes simultaneous installers create exactly one administrator.
pub async fn bootstrap_admin(pool: &DbPool) -> Result<Option<BootstrapCredentials>, CliError> {
    if sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE role='admin'")
        .fetch_one(pool)
        .await?
        > 0
    {
        sqlx::query("DELETE FROM settings WHERE key IN ('setup.first_run_token','setup.first_run_token_created_at')")
            .execute(pool).await?;
        return Ok(None);
    }
    let username = format!("admin_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let password = generate_admin_password();
    let to_hash = password.clone();
    let hash = tokio::task::spawn_blocking(move || hash_password(&to_hash))
        .await
        .map_err(|_| CliError::Other("Password hashing failed".into()))??;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role='admin'")
        .fetch_one(&mut *tx)
        .await?;
    if exists == 0 {
        sqlx::query("INSERT INTO users (username,password_hash,role,status,created_at,updated_at) VALUES (?,?,'admin','active',unixepoch(),unixepoch())")
            .bind(&username).bind(hash).execute(&mut *tx).await?;
    }
    sqlx::query("DELETE FROM settings WHERE key IN ('setup.first_run_token','setup.first_run_token_created_at')")
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((exists == 0).then_some(BootstrapCredentials { username, password }))
}

/// Validate that a username is suitable for a panel user.
pub fn validate_admin_username(username: &str) -> Result<(), CliError> {
    if username.len() < 3 || username.len() > 32 {
        return Err(CliError::Validation(
            "Username must be between 3 and 32 characters long".to_string(),
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(CliError::Validation(
            "Username may only contain alphanumeric characters, underscores, and hyphens"
                .to_string(),
        ));
    }
    Ok(())
}

/// Creates a new admin user.
pub async fn create_admin(
    pool: &DbPool,
    username: &str,
    password: &str,
    email: Option<String>,
) -> Result<i64, CliError> {
    validate_admin_username(username)?;

    let password_hash = hash_password(password)?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let insert = sqlx::query("INSERT INTO users (username,password_hash,role,email,status,created_at,updated_at) VALUES (?,?,'admin',?,'active',unixepoch(),unixepoch())")
        .bind(username).bind(password_hash).bind(email).execute(&mut *tx).await?;
    let id = insert.last_insert_rowid();
    sqlx::query("DELETE FROM settings WHERE key IN ('setup.first_run_token','setup.first_run_token_created_at')")
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(id)
}

/// Resets the password for an existing user.
pub async fn reset_password(
    pool: &DbPool,
    username: &str,
    new_password: &str,
) -> Result<(), CliError> {
    let user_repo = SqliteUserRepository::new(pool.clone());

    let user = user_repo
        .find_by_username(username)
        .await?
        .ok_or_else(|| CliError::NotFound(format!("User '{}' not found", username)))?;

    if user.role != "admin" {
        return Err(CliError::Validation(
            "Gunakan perintah ini untuk akun administrator.".into(),
        ));
    }
    let new_hash = hash_password(new_password)?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let updated = sqlx::query(
        "UPDATE users SET password_hash=?, updated_at=unixepoch() WHERE id=? AND role='admin'",
    )
    .bind(new_hash)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(CliError::Conflict(
            "Akun administrator telah berubah; ulangi perintah.".into(),
        ));
    }
    sqlx::query("DELETE FROM sessions WHERE user_id=?")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM api_tokens WHERE user_id=?")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(())
}

/// Lists all admin users.
pub async fn list_admins(pool: &DbPool) -> Result<Vec<panel_db::models::User>, CliError> {
    let user_repo = SqliteUserRepository::new(pool.clone());
    let users = user_repo.list_all().await?;
    let admins = users.into_iter().filter(|u| u.role == "admin").collect();
    Ok(admins)
}

/// Resets two-factor authentication for an account (emergency CLI rescue).
pub async fn reset_2fa(pool: &DbPool, username: &str) -> Result<(), CliError> {
    let user_repo = SqliteUserRepository::new(pool.clone());

    let user = user_repo
        .find_by_username(username)
        .await?
        .ok_or_else(|| CliError::NotFound(format!("User '{}' not found", username)))?;

    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query(
        "UPDATE users SET totp_secret=NULL, totp_enabled=0, updated_at=unixepoch() WHERE id=?",
    )
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM user_recovery_codes WHERE user_id=?")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id=?")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM api_tokens WHERE user_id=?")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
