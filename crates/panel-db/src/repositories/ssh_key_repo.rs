//! SSH Key repository for tenant and user public keys.

use crate::error::DbError;
use crate::models::{CreateSshKey, SshKey};
use crate::pool::DbPool;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqliteSshKeyRepository {
    pool: DbPool,
}

impl SqliteSshKeyRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, input: &CreateSshKey) -> Result<SshKey, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let result = sqlx::query(
            r#"
            INSERT INTO ssh_keys (user_id, name, public_key, key_type, fingerprint_sha256, fingerprint_md5, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(input.user_id)
        .bind(&input.name)
        .bind(&input.public_key)
        .bind(&input.key_type)
        .bind(&input.fingerprint_sha256)
        .bind(&input.fingerprint_md5)
        .bind(now)
        .execute(&self.pool)
        .await?;

        let id = result.last_insert_rowid();

        Ok(SshKey {
            id,
            user_id: input.user_id,
            name: input.name.clone(),
            public_key: input.public_key.clone(),
            key_type: input.key_type.clone(),
            fingerprint_sha256: input.fingerprint_sha256.clone(),
            fingerprint_md5: input.fingerprint_md5.clone(),
            created_at: now,
        })
    }

    pub async fn list_by_user(&self, user_id: i64) -> Result<Vec<SshKey>, DbError> {
        let keys = sqlx::query_as::<_, SshKey>(
            r#"
            SELECT id, user_id, name, public_key, key_type, fingerprint_sha256, fingerprint_md5, created_at
            FROM ssh_keys
            WHERE user_id = ?
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(keys)
    }

    pub async fn get_by_id(&self, id: i64) -> Result<Option<SshKey>, DbError> {
        let key = sqlx::query_as::<_, SshKey>(
            r#"
            SELECT id, user_id, name, public_key, key_type, fingerprint_sha256, fingerprint_md5, created_at
            FROM ssh_keys
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(key)
    }

    pub async fn find_by_user_and_fingerprint(
        &self,
        user_id: i64,
        fingerprint_sha256: &str,
    ) -> Result<Option<SshKey>, DbError> {
        let key = sqlx::query_as::<_, SshKey>(
            r#"
            SELECT id, user_id, name, public_key, key_type, fingerprint_sha256, fingerprint_md5, created_at
            FROM ssh_keys
            WHERE user_id = ? AND fingerprint_sha256 = ?
            "#,
        )
        .bind(user_id)
        .bind(fingerprint_sha256)
        .fetch_optional(&self.pool)
        .await?;

        Ok(key)
    }

    pub async fn delete(&self, id: i64, user_id: Option<i64>) -> Result<bool, DbError> {
        let result = if let Some(uid) = user_id {
            sqlx::query(
                r#"
                DELETE FROM ssh_keys
                WHERE id = ? AND user_id = ?
                "#,
            )
            .bind(id)
            .bind(uid)
            .execute(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                DELETE FROM ssh_keys
                WHERE id = ?
                "#,
            )
            .bind(id)
            .execute(&self.pool)
            .await?
        };

        Ok(result.rows_affected() > 0)
    }

    pub async fn list_all_for_linux_username(
        &self,
        linux_username: &str,
    ) -> Result<Vec<SshKey>, DbError> {
        let keys = sqlx::query_as::<_, SshKey>(
            r#"
            SELECT k.id, k.user_id, k.name, k.public_key, k.key_type, k.fingerprint_sha256, k.fingerprint_md5, k.created_at
            FROM ssh_keys k
            INNER JOIN users u ON u.id = k.user_id
            WHERE u.linux_username = ?
            ORDER BY k.created_at ASC
            "#,
        )
        .bind(linux_username)
        .fetch_all(&self.pool)
        .await?;

        Ok(keys)
    }
}
