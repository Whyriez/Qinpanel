//! Domain repository for domain mapping and management.

use crate::error::DbError;
use crate::models::{CreateDomain, Domain, UpdateDomain};
use crate::pool::DbPool;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqliteDomainRepository {
    pool: DbPool,
}

impl SqliteDomainRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Creates a new domain for a site.
    /// If `is_primary` is true, resets any previous primary domain for this site.
    pub async fn create(&self, input: &CreateDomain) -> Result<Domain, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let redirect_mode = input.redirect_mode.as_deref().unwrap_or("none");
        let status = input.status.as_deref().unwrap_or("active");

        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(DbError::Sqlx)?;

        if input.is_primary {
            // Demote other domains for this site
            sqlx::query("UPDATE domains SET is_primary = 0, updated_at = ? WHERE site_id = ?")
                .bind(now)
                .bind(input.site_id)
                .execute(&mut *tx)
                .await
                .map_err(DbError::Sqlx)?;
        }

        let res = sqlx::query(
            r#"
            INSERT INTO domains (
                site_id, name, unicode_name, is_primary,
                redirect_mode, redirect_target, status, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(input.site_id)
        .bind(&input.name)
        .bind(&input.unicode_name)
        .bind(input.is_primary)
        .bind(redirect_mode)
        .bind(&input.redirect_target)
        .bind(status)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(ref db_err) if db_err.is_unique_violation() => DbError::Conflict(
                format!("Domain '{}' already exists in the system", input.name),
            ),
            other => DbError::Sqlx(other),
        })?;

        let id = res.last_insert_rowid();

        tx.commit().await.map_err(DbError::Sqlx)?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Domain {} not found after insert", id)))
    }

    /// Finds a domain by ID.
    pub async fn find_by_id(&self, id: i64) -> Result<Option<Domain>, DbError> {
        sqlx::query_as::<_, Domain>(
            r#"
            SELECT id, site_id, name, unicode_name, is_primary,
                   redirect_mode, redirect_target, status, created_at, updated_at
            FROM domains
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Finds a domain by ASCII name or Unicode name.
    pub async fn find_by_name(&self, name: &str) -> Result<Option<Domain>, DbError> {
        let normalized = name.trim().to_ascii_lowercase();

        sqlx::query_as::<_, Domain>(
            r#"
            SELECT id, site_id, name, unicode_name, is_primary,
                   redirect_mode, redirect_target, status, created_at, updated_at
            FROM domains
            WHERE name = ? OR unicode_name = ?
            LIMIT 1
            "#,
        )
        .bind(&normalized)
        .bind(name.trim())
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Lists all domains associated with a website.
    pub async fn find_by_site_id(&self, site_id: i64) -> Result<Vec<Domain>, DbError> {
        sqlx::query_as::<_, Domain>(
            r#"
            SELECT id, site_id, name, unicode_name, is_primary,
                   redirect_mode, redirect_target, status, created_at, updated_at
            FROM domains
            WHERE site_id = ?
            ORDER BY is_primary DESC, name ASC
            "#,
        )
        .bind(site_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Finds the designated primary domain for a site.
    pub async fn find_primary_by_site_id(&self, site_id: i64) -> Result<Option<Domain>, DbError> {
        sqlx::query_as::<_, Domain>(
            r#"
            SELECT id, site_id, name, unicode_name, is_primary,
                   redirect_mode, redirect_target, status, created_at, updated_at
            FROM domains
            WHERE site_id = ? AND is_primary = 1
            LIMIT 1
            "#,
        )
        .bind(site_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Sets a specific domain as the primary domain for a website, demoting any existing primary.
    pub async fn set_primary(&self, site_id: i64, domain_id: i64) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(DbError::Sqlx)?;

        // Demote all
        sqlx::query("UPDATE domains SET is_primary = 0, updated_at = ? WHERE site_id = ?")
            .bind(now)
            .bind(site_id)
            .execute(&mut *tx)
            .await
            .map_err(DbError::Sqlx)?;

        // Promote target
        let rows = sqlx::query(
            "UPDATE domains SET is_primary = 1, updated_at = ? WHERE id = ? AND site_id = ?",
        )
        .bind(now)
        .bind(domain_id)
        .bind(site_id)
        .execute(&mut *tx)
        .await
        .map_err(DbError::Sqlx)?
        .rows_affected();

        if rows == 0 {
            return Err(DbError::NotFound(format!(
                "Domain {} not found for site {}",
                domain_id, site_id
            )));
        }

        tx.commit().await.map_err(DbError::Sqlx)?;
        Ok(())
    }

    /// Updates domain settings.
    pub async fn update(&self, id: i64, input: &UpdateDomain) -> Result<Domain, DbError> {
        let existing = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Domain {} not found for update", id)))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let name = input.name.as_ref().unwrap_or(&existing.name);
        let unicode_name = input
            .unicode_name
            .as_ref()
            .unwrap_or(&existing.unicode_name);
        let is_primary = input.is_primary.unwrap_or(existing.is_primary);
        let redirect_mode = input
            .redirect_mode
            .as_ref()
            .unwrap_or(&existing.redirect_mode);
        let redirect_target = input
            .redirect_target
            .as_ref()
            .or(existing.redirect_target.as_ref());
        let status = input.status.as_ref().unwrap_or(&existing.status);

        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(DbError::Sqlx)?;

        if is_primary && !existing.is_primary {
            sqlx::query("UPDATE domains SET is_primary = 0, updated_at = ? WHERE site_id = ?")
                .bind(now)
                .bind(existing.site_id)
                .execute(&mut *tx)
                .await
                .map_err(DbError::Sqlx)?;
        }

        sqlx::query(
            r#"
            UPDATE domains
            SET name = ?, unicode_name = ?, is_primary = ?,
                redirect_mode = ?, redirect_target = ?, status = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(name)
        .bind(unicode_name)
        .bind(is_primary)
        .bind(redirect_mode)
        .bind(redirect_target)
        .bind(status)
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(ref db_err) if db_err.is_unique_violation() => {
                DbError::Conflict(format!("Domain '{}' already exists in the system", name))
            }
            other => DbError::Sqlx(other),
        })?;

        tx.commit().await.map_err(DbError::Sqlx)?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Domain {} not found after update", id)))
    }

    /// Updates domain status ('active', 'pending_ssl', 'disabled').
    pub async fn update_status(&self, id: i64, status: &str) -> Result<Domain, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let rows = sqlx::query("UPDATE domains SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?
            .rows_affected();

        if rows == 0 {
            return Err(DbError::NotFound(format!("Domain {} not found", id)));
        }

        self.find_by_id(id).await?.ok_or_else(|| {
            DbError::NotFound(format!("Domain {} not found after status update", id))
        })
    }

    /// Deletes a domain.
    pub async fn delete(&self, id: i64) -> Result<bool, DbError> {
        let res = sqlx::query("DELETE FROM domains WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;

        Ok(res.rows_affected() > 0)
    }

    /// Counts domains mapped to a specific site.
    pub async fn count_by_site_id(&self, site_id: i64) -> Result<i64, DbError> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM domains WHERE site_id = ?")
            .bind(site_id)
            .fetch_one(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;

        Ok(count.0)
    }

    /// Counts total domains owned across all sites of a user.
    pub async fn count_by_user_id(&self, user_id: i64) -> Result<i64, DbError> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(d.id)
            FROM domains d
            INNER JOIN sites s ON d.site_id = s.id
            WHERE s.user_id = ?
            "#,
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(count.0)
    }

    /// Checks if a domain is available globally across the system.
    pub async fn is_domain_available(&self, name: &str) -> Result<bool, DbError> {
        let normalized = name.trim().to_ascii_lowercase();

        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM domains WHERE name = ? OR unicode_name = ?")
                .bind(&normalized)
                .bind(name.trim())
                .fetch_one(&self.pool)
                .await
                .map_err(DbError::Sqlx)?;

        Ok(count.0 == 0)
    }
}
