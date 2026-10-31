//! User repository for user data management.

use crate::error::DbError;
use crate::models::{CreateUser, User};
use crate::pool::DbPool;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqliteUserRepository {
    pool: DbPool,
}

impl SqliteUserRepository {
    /// The enrollment endpoint cannot replace an already active factor, even
    /// with concurrent requests that authenticated before the first enrollment.
    pub async fn enable_totp_if_disabled(
        &self,
        id: i64,
        session_id: &str,
        secret: &str,
        hashes: &[String],
    ) -> Result<bool, DbError> {
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let changed = sqlx::query("UPDATE users SET totp_secret=?,totp_enabled=1,updated_at=? WHERE id=? AND status='active' AND totp_enabled=0 AND EXISTS(SELECT 1 FROM sessions WHERE id=? AND user_id=users.id AND expires_at>?)")
            .bind(secret).bind(now).bind(id).bind(session_id).bind(now).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Ok(false);
        }
        sqlx::query("DELETE FROM user_recovery_codes WHERE user_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        for hash in hashes {
            sqlx::query("INSERT INTO user_recovery_codes(user_id,code_hash,used_at,created_at) VALUES(?,?,NULL,?)")
                .bind(id).bind(hash).bind(now).execute(&mut *tx).await?;
        }
        sqlx::query("DELETE FROM sessions WHERE user_id=? AND id<>?")
            .bind(id)
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, input: &CreateUser) -> Result<User, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let res = sqlx::query(
            r#"
            INSERT INTO users (
                username, password_hash, role, email, status,
                linux_username, linux_uid, plan_id, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, 'active', ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&input.username)
        .bind(&input.password_hash)
        .bind(&input.role)
        .bind(&input.email)
        .bind(&input.linux_username)
        .bind(input.linux_uid)
        .bind(input.plan_id)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(ref db_err) if db_err.is_unique_violation() => {
                DbError::Conflict(format!(
                    "Username or Linux username already exists: {}",
                    input.username
                ))
            }
            other => DbError::Sqlx(other),
        })?;

        let id = res.last_insert_rowid();

        self.find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("User {} not found after insert", id)))
    }

    pub async fn find_by_id(&self, id: i64) -> Result<Option<User>, DbError> {
        let user = sqlx::query_as::<_, User>(
            r#"
            SELECT
                id, username, password_hash, role, email, totp_secret,
                totp_enabled, status, linux_username, linux_uid, plan_id,
                shell_policy, created_at, updated_at
            FROM users
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(user)
    }

    pub async fn find_by_username(&self, username: &str) -> Result<Option<User>, DbError> {
        let user = sqlx::query_as::<_, User>(
            r#"
            SELECT
                id, username, password_hash, role, email, totp_secret,
                totp_enabled, status, linux_username, linux_uid, plan_id,
                shell_policy, created_at, updated_at
            FROM users
            WHERE username = ?
            "#,
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;

        Ok(user)
    }

    pub async fn find_by_linux_username(
        &self,
        linux_username: &str,
    ) -> Result<Option<User>, DbError> {
        let user = sqlx::query_as::<_, User>(
            r#"
            SELECT
                id, username, password_hash, role, email, totp_secret,
                totp_enabled, status, linux_username, linux_uid, plan_id,
                shell_policy, created_at, updated_at
            FROM users
            WHERE linux_username = ?
            "#,
        )
        .bind(linux_username)
        .fetch_optional(&self.pool)
        .await?;

        Ok(user)
    }

    pub async fn update_password(&self, id: i64, new_hash: &str) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let res = sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
            .bind(new_hash)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;

        if res.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("User with id {} not found", id)));
        }
        Ok(())
    }

    pub async fn update_totp(
        &self,
        id: i64,
        secret: Option<&str>,
        enabled: bool,
    ) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let totp_int = if enabled { 1 } else { 0 };
        let res = sqlx::query(
            "UPDATE users SET totp_secret = ?, totp_enabled = ?, updated_at = ? WHERE id = ?",
        )
        .bind(secret)
        .bind(totp_int)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        if res.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("User with id {} not found", id)));
        }
        Ok(())
    }

    pub async fn update_status(&self, id: i64, status: &str) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let res = sqlx::query("UPDATE users SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;

        if res.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("User with id {} not found", id)));
        }
        Ok(())
    }

    pub async fn list_all(&self) -> Result<Vec<User>, DbError> {
        let users = sqlx::query_as::<_, User>(
            r#"
            SELECT
                id, username, password_hash, role, email, totp_secret,
                totp_enabled, status, linux_username, linux_uid, plan_id,
                shell_policy, created_at, updated_at
            FROM users
            ORDER BY id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(users)
    }

    pub async fn update_shell_policy(
        &self,
        id: i64,
        shell_policy: Option<&str>,
    ) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let res = sqlx::query("UPDATE users SET shell_policy = ?, updated_at = ? WHERE id = ?")
            .bind(shell_policy)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;

        if res.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("User with id {} not found", id)));
        }
        Ok(())
    }

    pub async fn delete(&self, id: i64) -> Result<(), DbError> {
        let res = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if res.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("User with id {} not found", id)));
        }
        Ok(())
    }

    /// Allocates the next available Linux tenant UID in the range 10000..=59999.
    pub async fn allocate_next_uid(&self) -> Result<u32, DbError> {
        let max_uid: Option<i64> =
            sqlx::query_scalar("SELECT MAX(linux_uid) FROM users WHERE linux_uid IS NOT NULL")
                .fetch_one(&self.pool)
                .await?;

        let next = match max_uid {
            Some(curr) => (curr + 1) as u32,
            None => 10000,
        };

        if next < 10000 {
            Ok(10000)
        } else if next > 59999 {
            Err(DbError::Validation(
                "Maximum tenant UID 59999 reached".to_string(),
            ))
        } else {
            Ok(next)
        }
    }
}
