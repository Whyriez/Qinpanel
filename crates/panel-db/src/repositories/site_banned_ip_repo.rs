//! Site banned IP repository for security blocking with TTL.

use crate::error::DbError;
use crate::models::{CreateSiteBannedIpInput, SiteBannedIpModel};
use crate::pool::DbPool;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Clone)]
pub struct SqliteSiteBannedIpRepository {
    pool: DbPool,
}

impl SqliteSiteBannedIpRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Records a new banned IP for a site with optional TTL.
    pub async fn create(
        &self,
        input: &CreateSiteBannedIpInput,
    ) -> Result<SiteBannedIpModel, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let id = input
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        // Deactivate existing active entries for this (site_id, ip) first
        let _ = sqlx::query(
            "UPDATE site_banned_ips SET is_active = 0, updated_at = ? WHERE site_id = ? AND ip_address = ?",
        )
        .bind(now)
        .bind(input.site_id)
        .bind(&input.ip_address)
        .execute(&self.pool)
        .await;

        sqlx::query(
            r#"
            INSERT INTO site_banned_ips (
                id, site_id, ip_address, reason, banned_at, expires_at,
                banned_by, is_active, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(input.site_id)
        .bind(&input.ip_address)
        .bind(&input.reason)
        .bind(input.banned_at)
        .bind(input.expires_at)
        .bind(&input.banned_by)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        self.find_by_id(&id).await?.ok_or_else(|| {
            DbError::NotFound(format!("Banned IP record {} not found after insert", id))
        })
    }

    /// Finds a banned IP record by primary key ID.
    pub async fn find_by_id(&self, id: &str) -> Result<Option<SiteBannedIpModel>, DbError> {
        sqlx::query_as::<_, SiteBannedIpModel>("SELECT * FROM site_banned_ips WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Sqlx)
    }

    /// Finds active banned IP record for a site.
    pub async fn find_active(
        &self,
        site_id: i64,
        ip_address: &str,
        current_ts: i64,
    ) -> Result<Option<SiteBannedIpModel>, DbError> {
        sqlx::query_as::<_, SiteBannedIpModel>(
            r#"
            SELECT * FROM site_banned_ips
            WHERE site_id = ? AND ip_address = ? AND is_active = 1
              AND (expires_at IS NULL OR expires_at > ?)
            ORDER BY banned_at DESC
            LIMIT 1
            "#,
        )
        .bind(site_id)
        .bind(ip_address)
        .bind(current_ts)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Lists all active and unexpired banned IPs for a specific site.
    pub async fn list_active_by_site(
        &self,
        site_id: i64,
        current_ts: i64,
    ) -> Result<Vec<SiteBannedIpModel>, DbError> {
        sqlx::query_as::<_, SiteBannedIpModel>(
            r#"
            SELECT * FROM site_banned_ips
            WHERE site_id = ? AND is_active = 1
              AND (expires_at IS NULL OR expires_at > ?)
            ORDER BY banned_at DESC
            "#,
        )
        .bind(site_id)
        .bind(current_ts)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Unbans an IP address for a site by marking its records inactive.
    pub async fn unban(&self, site_id: i64, ip_address: &str) -> Result<u64, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let res = sqlx::query(
            "UPDATE site_banned_ips SET is_active = 0, updated_at = ? WHERE site_id = ? AND ip_address = ? AND is_active = 1",
        )
        .bind(now)
        .bind(site_id)
        .bind(ip_address)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(res.rows_affected())
    }

    /// Deactivates expired bans and returns the list of deactivated records.
    pub async fn deactivate_expired(
        &self,
        current_ts: i64,
    ) -> Result<Vec<SiteBannedIpModel>, DbError> {
        let expired: Vec<SiteBannedIpModel> = sqlx::query_as::<_, SiteBannedIpModel>(
            "SELECT * FROM site_banned_ips WHERE is_active = 1 AND expires_at IS NOT NULL AND expires_at <= ?",
        )
        .bind(current_ts)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        if !expired.is_empty() {
            sqlx::query(
                "UPDATE site_banned_ips SET is_active = 0, updated_at = ? WHERE is_active = 1 AND expires_at IS NOT NULL AND expires_at <= ?",
            )
            .bind(current_ts)
            .bind(current_ts)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;
        }

        Ok(expired)
    }
}
