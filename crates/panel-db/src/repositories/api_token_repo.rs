//! API Token repository for scoped machine/CLI access tokens.

use crate::error::DbError;
use crate::models::{ApiToken, CreateApiToken};
use crate::pool::DbPool;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqliteApiTokenRepository {
    pool: DbPool,
}

impl SqliteApiTokenRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, input: &CreateApiToken) -> Result<ApiToken, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        sqlx::query(
            r#"
            INSERT INTO api_tokens (id, user_id, name, token_hash, scopes_json, expires_at, last_used_at, created_at)
            VALUES (?, ?, ?, ?, ?, ?, NULL, ?)
            "#,
        )
        .bind(&input.id)
        .bind(input.user_id)
        .bind(&input.name)
        .bind(&input.token_hash)
        .bind(&input.scopes_json)
        .bind(input.expires_at)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(ApiToken {
            id: input.id.clone(),
            user_id: input.user_id,
            name: input.name.clone(),
            token_hash: input.token_hash.clone(),
            scopes_json: input.scopes_json.clone(),
            expires_at: input.expires_at,
            last_used_at: None,
            created_at: now,
        })
    }

    pub async fn find_by_token_hash(&self, token_hash: &str) -> Result<Option<ApiToken>, DbError> {
        let token = sqlx::query_as::<_, ApiToken>(
            r#"
            SELECT id, user_id, name, token_hash, scopes_json, expires_at, last_used_at, created_at
            FROM api_tokens
            WHERE token_hash = ?
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(token)
    }

    pub async fn list_by_user_id(&self, user_id: i64) -> Result<Vec<ApiToken>, DbError> {
        let tokens = sqlx::query_as::<_, ApiToken>(
            r#"
            SELECT id, user_id, name, token_hash, scopes_json, expires_at, last_used_at, created_at
            FROM api_tokens
            WHERE user_id = ?
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(tokens)
    }

    pub async fn update_last_used(&self, id: &str, last_used_at: i64) -> Result<(), DbError> {
        let res = sqlx::query("UPDATE api_tokens SET last_used_at = ? WHERE id = ?")
            .bind(last_used_at)
            .bind(id)
            .execute(&self.pool)
            .await?;

        if res.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("ApiToken {} not found", id)));
        }
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<(), DbError> {
        let res = sqlx::query("DELETE FROM api_tokens WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if res.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("ApiToken {} not found", id)));
        }
        Ok(())
    }
}
