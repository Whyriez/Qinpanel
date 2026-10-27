//! Setting repository for key-value configuration.

use crate::error::DbError;
use crate::models::Setting;
use crate::pool::DbPool;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqliteSettingRepository {
    pool: DbPool,
}

impl SqliteSettingRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, key: &str) -> Result<Option<String>, DbError> {
        let record = sqlx::query_as::<_, Setting>(
            "SELECT key, value, updated_at FROM settings WHERE key = ?",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(record.map(|r| r.value))
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        sqlx::query(
            r#"
            INSERT INTO settings (key, value, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(key)
        .bind(value)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn delete(&self, key: &str) -> Result<(), DbError> {
        let res = sqlx::query("DELETE FROM settings WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await?;

        if res.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Setting key {} not found", key)));
        }
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<Setting>, DbError> {
        let settings = sqlx::query_as::<_, Setting>(
            "SELECT key, value, updated_at FROM settings ORDER BY key ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(settings)
    }
}
