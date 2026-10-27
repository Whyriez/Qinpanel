//! Database connection pool initialization and migrations.

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::str::FromStr;
use std::time::Duration;
use tracing::info;

use crate::error::DbError;

pub type DbPool = SqlitePool;

/// WAL keeps reads concurrent with writes; a busy timeout also covers other processes.
pub async fn init_pool(database_url: &str) -> Result<DbPool, DbError> {
    init_pool_with_limit(database_url, 8).await
}

pub async fn init_pool_with_limit(database_url: &str, connections: u32) -> Result<DbPool, DbError> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(15))
        .pragma("wal_autocheckpoint", "1000");

    let pool = SqlitePoolOptions::new()
        .max_connections(connections)
        .acquire_timeout(Duration::from_secs(30))
        .connect_with(options)
        .await?;

    info!("SQLite database connection pool initialized successfully");
    Ok(pool)
}

/// Runs embedded database migrations.
pub async fn run_migrations(pool: &DbPool) -> Result<(), DbError> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(DbError::Migrate)?;
    info!("Database migrations applied successfully");
    Ok(())
}
