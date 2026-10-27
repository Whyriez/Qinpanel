//! Independent lock domain for jobs and audit. Paths derive from the actual core
//! connection, so API, CLI and backups cannot silently choose different stores.

use crate::DbError;
use crate::pool::DbPool;
use sqlx::Connection;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn sidecar_path(core: &Path) -> PathBuf {
    let mut name = core.file_stem().unwrap_or_default().to_os_string();
    name.push(".operations.db");
    core.with_file_name(name)
}

pub fn same_file_path(left: &Path, right: &Path) -> bool {
    fn resolved(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| {
            let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
            match (absolute.parent(), absolute.file_name()) {
                (Some(parent), Some(name)) => parent
                    .canonicalize()
                    .map(|parent| parent.join(name))
                    .unwrap_or(absolute),
                _ => absolute,
            }
        })
    }
    resolved(left) == resolved(right)
}

pub async fn database_path(pool: &DbPool) -> Result<Option<PathBuf>, DbError> {
    let rows: Vec<(i64, String, String)> = sqlx::query_as("PRAGMA database_list")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().find_map(|(_, name, file)| {
        (name == "main" && !file.is_empty()).then(|| PathBuf::from(file))
    }))
}

/// A single connection is the FIFO writer queue. Readers use their own pool and
/// SQLite query_only prevents accidental writes from bypassing that queue.
pub async fn open(core: &DbPool) -> Result<(DbPool, DbPool), DbError> {
    let Some(source) = database_path(core).await? else {
        // In-memory fixtures intentionally retain a single, isolated database.
        return Ok((core.clone(), core.clone()));
    };
    let path = sidecar_path(&source);
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Full)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(15));
    let writer = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(30))
        // Only the startup writer establishes persistent WAL mode. In particular,
        // a restored VACUUM INTO snapshot starts in rollback-journal mode.
        .connect_with(
            options
                .clone()
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal),
        )
        .await?;
    sqlx::migrate!("./operations_migrations")
        .run(&writer)
        .await?;
    import_legacy(&writer, &source).await?;
    let reader = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(30))
        // Inherit the writer's mode. Reissuing journal_mode on a reader can need
        // an exclusive lock while the writer already owns this database.
        .connect_with(options.create_if_missing(false).pragma("query_only", "ON"))
        .await?;
    Ok((writer, reader))
}

async fn import_legacy(writer: &DbPool, source: &Path) -> Result<(), DbError> {
    let mut connection = writer.acquire().await?;
    let imported: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM legacy_import")
        .fetch_one(&mut *connection)
        .await?;
    if imported != 0 {
        return Ok(());
    }
    sqlx::query("ATTACH DATABASE ? AS legacy")
        .bind(source.to_string_lossy().as_ref())
        .execute(&mut *connection)
        .await?;
    let result: Result<(), DbError> = async {
        // Import before workers/listeners start. Only the destination is mutated;
        // copy and marker commit together, so interruption can safely be retried.
        let mut tx = connection.begin_with("BEGIN IMMEDIATE").await?;
        let imported: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM legacy_import")
            .fetch_one(&mut *tx)
            .await?;
        if imported == 0 {
            sqlx::query("INSERT INTO jobs SELECT * FROM legacy.jobs")
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO audit_logs SELECT * FROM legacy.audit_logs")
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO legacy_import(id, source_path) VALUES (1, ?)")
                .bind(source.to_string_lossy().as_ref())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    .await;
    // Never return a connection with an attached core DB to the writer queue.
    let detached = sqlx::query("DETACH DATABASE legacy")
        .execute(&mut *connection)
        .await;
    if detached.is_err() {
        connection.close().await?;
    }
    result?;
    detached?;
    Ok(())
}

/// Open the already-imported operational store for out-of-band maintenance.
/// Callers own/close the returned pool; this does not change the core pool.
pub async fn for_maintenance(core: &DbPool) -> Result<DbPool, DbError> {
    let (writer, reader) = open(core).await?;
    if database_path(core).await?.is_some() {
        reader.close().await;
    }
    Ok(writer)
}
