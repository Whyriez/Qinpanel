//! SQLite snapshots include committed WAL contents. Copying the main file after
//! a checkpoint is unsafe when a writer can resume between checkpoint and copy.

use crate::{DbError, DbPool};
use sqlx::Connection;
use std::path::Path;
use std::time::Duration;

pub async fn from_pool(pool: &DbPool, destination: &Path) -> Result<(), DbError> {
    sqlx::query("VACUUM main INTO ?")
        .bind(destination.to_string_lossy().as_ref())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn from_file(source: &Path, destination: &Path) -> Result<(), DbError> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(source)
        .read_only(true)
        .busy_timeout(Duration::from_secs(15));
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await?;
    let result = sqlx::query("VACUUM main INTO ?")
        .bind(destination.to_string_lossy().as_ref())
        .execute(&mut connection)
        .await;
    connection.close().await?;
    result?;
    Ok(())
}

/// The agent's backup manager is synchronous and may be called inside a Tokio
/// worker. Use a separate thread/runtime rather than nesting block_on there.
pub fn from_file_blocking(source: &Path, destination: &Path) -> Result<(), DbError> {
    let source = source.to_path_buf();
    let destination = destination.to_path_buf();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(sqlx::Error::Io)?;
        runtime.block_on(from_file(&source, &destination))
    })
    .join()
    .map_err(|_| DbError::Validation("Database snapshot worker panicked".to_owned()))?
}
