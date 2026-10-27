//! Recovery code repository for 2FA recovery codes.

use crate::error::DbError;
use crate::pool::DbPool;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqliteRecoveryCodeRepository {
    pool: DbPool,
}

impl SqliteRecoveryCodeRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Replaces existing recovery codes for a user with a new set of hashed codes.
    pub async fn replace_codes(&self, user_id: i64, code_hashes: &[String]) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        // 1. Delete old codes
        sqlx::query("DELETE FROM user_recovery_codes WHERE user_id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        // 2. Insert new codes
        for hash in code_hashes {
            sqlx::query(
                r#"
                INSERT INTO user_recovery_codes (user_id, code_hash, used_at, created_at)
                VALUES (?, ?, NULL, ?)
                "#,
            )
            .bind(user_id)
            .bind(hash)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Attempts to consume an unused recovery code matching `code_hash`.
    /// Returns `true` if a code was successfully matched and consumed, `false` otherwise.
    pub async fn verify_and_consume(&self, user_id: i64, code_hash: &str) -> Result<bool, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let res = sqlx::query(
            r#"
            UPDATE user_recovery_codes
            SET used_at = ?
            WHERE id = (
                SELECT id FROM user_recovery_codes
                WHERE user_id = ? AND code_hash = ? AND used_at IS NULL
                LIMIT 1
            )
            "#,
        )
        .bind(now)
        .bind(user_id)
        .bind(code_hash)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() > 0)
    }

    /// Counts how many unused recovery codes remain for a user.
    pub async fn count_unused(&self, user_id: i64) -> Result<i64, DbError> {
        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM user_recovery_codes
            WHERE user_id = ? AND used_at IS NULL
            "#,
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0)
    }

    /// Deletes all recovery codes for a user (e.g. when 2FA is disabled).
    pub async fn delete_by_user_id(&self, user_id: i64) -> Result<(), DbError> {
        sqlx::query("DELETE FROM user_recovery_codes WHERE user_id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
