//! Site repository for website management.

use crate::error::DbError;
use crate::models::{CreateSite, Site, UpdateSite};
use crate::pool::DbPool;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqliteSiteRepository {
    pool: DbPool,
}

impl SqliteSiteRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Creates a new website entry in the database.
    pub async fn create(&self, input: &CreateSite) -> Result<Site, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let status = input.status.as_deref().unwrap_or("active");
        let settings_json = input.settings_json.as_deref().unwrap_or("{}");

        let res = sqlx::query(
            r#"
            INSERT INTO sites (
                user_id, kind, docroot, runtime, version,
                status, settings_json, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(input.user_id)
        .bind(&input.kind)
        .bind(&input.docroot)
        .bind(&input.runtime)
        .bind(&input.version)
        .bind(status)
        .bind(settings_json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let id = res.last_insert_rowid();

        self.find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Site {} not found after insert", id)))
    }

    /// Finds a website by its primary key ID.
    pub async fn find_by_id(&self, id: i64) -> Result<Option<Site>, DbError> {
        sqlx::query_as::<_, Site>(
            r#"
            SELECT id, user_id, kind, docroot, runtime, version,
                   status, settings_json, created_at, updated_at
            FROM sites
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Lists all websites belonging to a specific tenant user.
    pub async fn find_by_user_id(&self, user_id: i64) -> Result<Vec<Site>, DbError> {
        sqlx::query_as::<_, Site>(
            r#"
            SELECT id, user_id, kind, docroot, runtime, version,
                   status, settings_json, created_at, updated_at
            FROM sites
            WHERE user_id = ?
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Lists all websites across all users (for system administration).
    pub async fn list_all(&self) -> Result<Vec<Site>, DbError> {
        sqlx::query_as::<_, Site>(
            r#"
            SELECT id, user_id, kind, docroot, runtime, version,
                   status, settings_json, created_at, updated_at
            FROM sites
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Updates site settings and configurations.
    pub async fn update(&self, id: i64, input: &UpdateSite) -> Result<Site, DbError> {
        let existing = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Site {} not found for update", id)))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let kind = input.kind.as_ref().unwrap_or(&existing.kind);
        let docroot = input.docroot.as_ref().unwrap_or(&existing.docroot);
        let runtime = input.runtime.as_ref().or(existing.runtime.as_ref());
        let version = input.version.as_ref().or(existing.version.as_ref());
        let status = input.status.as_ref().unwrap_or(&existing.status);
        let settings_json = input
            .settings_json
            .as_ref()
            .unwrap_or(&existing.settings_json);

        sqlx::query(
            r#"
            UPDATE sites
            SET kind = ?, docroot = ?, runtime = ?, version = ?,
                status = ?, settings_json = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(kind)
        .bind(docroot)
        .bind(runtime)
        .bind(version)
        .bind(status)
        .bind(settings_json)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Site {} not found after update", id)))
    }

    /// Updates the operational status of a site ('active', 'disabled', 'suspended').
    pub async fn update_status(&self, id: i64, status: &str) -> Result<Site, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let rows_affected = sqlx::query(
            r#"
            UPDATE sites
            SET status = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(status)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?
        .rows_affected();

        if rows_affected == 0 {
            return Err(DbError::NotFound(format!("Site {} not found", id)));
        }

        self.find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Site {} not found after status update", id)))
    }

    /// Deletes a website entry (cascades to all associated domains).
    pub async fn delete(&self, id: i64) -> Result<bool, DbError> {
        let res = sqlx::query("DELETE FROM sites WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;

        Ok(res.rows_affected() > 0)
    }

    /// Counts total sites owned by a specific user.
    pub async fn count_by_user_id(&self, user_id: i64) -> Result<i64, DbError> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sites WHERE user_id = ?")
            .bind(user_id)
            .fetch_one(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;

        Ok(count.0)
    }

    /// Counts sites configured with a specific runtime and version.
    pub async fn count_by_runtime_and_version(
        &self,
        runtime: &str,
        version: &str,
    ) -> Result<i64, DbError> {
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sites WHERE runtime = ? AND version = ?")
                .bind(runtime)
                .bind(version)
                .fetch_one(&self.pool)
                .await
                .map_err(DbError::Sqlx)?;

        Ok(count.0)
    }
}
