//! SQLite repository for site backups and automated backup schedules.

use crate::error::DbError;
use crate::models::{
    CreateBackupInput, SiteBackupModel, SiteBackupScheduleModel, UpsertBackupScheduleInput,
};
use crate::pool::DbPool;
use sqlx::Row;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqliteBackupRepository {
    pool: DbPool,
}

impl SqliteBackupRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Creates a new backup record in the database.
    pub async fn create_backup(
        &self,
        input: &CreateBackupInput,
    ) -> Result<SiteBackupModel, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let includes_files = if input.includes_files { 1 } else { 0 };
        let includes_databases = if input.includes_databases { 1 } else { 0 };

        let id = sqlx::query(
            r#"
            INSERT INTO site_backups (
                site_id, user_id, filename, filepath, size_bytes, label,
                includes_files, includes_databases, status, error_message,
                storage_type, s3_key, checksum_sha256, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(input.site_id)
        .bind(input.user_id)
        .bind(&input.filename)
        .bind(&input.filepath)
        .bind(input.size_bytes)
        .bind(&input.label)
        .bind(includes_files)
        .bind(includes_databases)
        .bind(&input.status)
        .bind(&input.storage_type)
        .bind(&input.s3_key)
        .bind(&input.checksum_sha256)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?
        .last_insert_rowid();

        Ok(SiteBackupModel {
            id,
            site_id: input.site_id,
            user_id: input.user_id,
            filename: input.filename.clone(),
            filepath: input.filepath.clone(),
            size_bytes: input.size_bytes,
            label: input.label.clone(),
            includes_files: input.includes_files,
            includes_databases: input.includes_databases,
            status: input.status.clone(),
            error_message: None,
            storage_type: input.storage_type.clone(),
            s3_key: input.s3_key.clone(),
            checksum_sha256: input.checksum_sha256.clone(),
            created_at: now,
            updated_at: now,
        })
    }

    /// Finds a backup record by its ID.
    pub async fn find_backup_by_id(&self, id: i64) -> Result<Option<SiteBackupModel>, DbError> {
        let row = sqlx::query(
            r#"
            SELECT
                id, site_id, user_id, filename, filepath, size_bytes, label,
                includes_files, includes_databases, status, error_message,
                storage_type, s3_key, checksum_sha256, created_at, updated_at
            FROM site_backups
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let model = row.map(|r| SiteBackupModel {
            id: r.get("id"),
            site_id: r.get("site_id"),
            user_id: r.get("user_id"),
            filename: r.get("filename"),
            filepath: r.get("filepath"),
            size_bytes: r.get("size_bytes"),
            label: r.get("label"),
            includes_files: r.get::<i64, _>("includes_files") != 0,
            includes_databases: r.get::<i64, _>("includes_databases") != 0,
            status: r.get("status"),
            error_message: r.get("error_message"),
            storage_type: r.get("storage_type"),
            s3_key: r.get("s3_key"),
            checksum_sha256: r.get("checksum_sha256"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        });

        Ok(model)
    }

    /// Lists all backup records for a given site, ordered by newest first.
    pub async fn list_backups_by_site(
        &self,
        site_id: i64,
    ) -> Result<Vec<SiteBackupModel>, DbError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, site_id, user_id, filename, filepath, size_bytes, label,
                includes_files, includes_databases, status, error_message,
                storage_type, s3_key, checksum_sha256, created_at, updated_at
            FROM site_backups
            WHERE site_id = ?
            ORDER BY created_at DESC
            "#,
        )
        .bind(site_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let list = rows
            .into_iter()
            .map(|r| SiteBackupModel {
                id: r.get("id"),
                site_id: r.get("site_id"),
                user_id: r.get("user_id"),
                filename: r.get("filename"),
                filepath: r.get("filepath"),
                size_bytes: r.get("size_bytes"),
                label: r.get("label"),
                includes_files: r.get::<i64, _>("includes_files") != 0,
                includes_databases: r.get::<i64, _>("includes_databases") != 0,
                status: r.get("status"),
                error_message: r.get("error_message"),
                storage_type: r.get("storage_type"),
                s3_key: r.get("s3_key"),
                checksum_sha256: r.get("checksum_sha256"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            })
            .collect();

        Ok(list)
    }

    /// Lists all backup records for a given user (tenant).
    pub async fn list_backups_by_user(
        &self,
        user_id: i64,
    ) -> Result<Vec<SiteBackupModel>, DbError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, site_id, user_id, filename, filepath, size_bytes, label,
                includes_files, includes_databases, status, error_message,
                storage_type, s3_key, checksum_sha256, created_at, updated_at
            FROM site_backups
            WHERE user_id = ?
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let list = rows
            .into_iter()
            .map(|r| SiteBackupModel {
                id: r.get("id"),
                site_id: r.get("site_id"),
                user_id: r.get("user_id"),
                filename: r.get("filename"),
                filepath: r.get("filepath"),
                size_bytes: r.get("size_bytes"),
                label: r.get("label"),
                includes_files: r.get::<i64, _>("includes_files") != 0,
                includes_databases: r.get::<i64, _>("includes_databases") != 0,
                status: r.get("status"),
                error_message: r.get("error_message"),
                storage_type: r.get("storage_type"),
                s3_key: r.get("s3_key"),
                checksum_sha256: r.get("checksum_sha256"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            })
            .collect();

        Ok(list)
    }

    /// Updates status, size, checksum, and error message of a backup.
    pub async fn update_backup_status(
        &self,
        id: i64,
        status: &str,
        size_bytes: Option<i64>,
        checksum_sha256: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        if let (Some(size), Some(chk)) = (size_bytes, checksum_sha256) {
            sqlx::query(
                r#"
                UPDATE site_backups
                SET status = ?, size_bytes = ?, checksum_sha256 = ?, error_message = ?, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(status)
            .bind(size)
            .bind(chk)
            .bind(error_message)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;
        } else {
            sqlx::query(
                r#"
                UPDATE site_backups
                SET status = ?, error_message = ?, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(status)
            .bind(error_message)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;
        }

        Ok(())
    }

    /// Deletes a backup record by ID.
    pub async fn delete_backup(&self, id: i64) -> Result<bool, DbError> {
        let affected = sqlx::query("DELETE FROM site_backups WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?
            .rows_affected();

        Ok(affected > 0)
    }

    /// Calculates total storage used by backups for a user.
    pub async fn get_total_backup_bytes_by_user(&self, user_id: i64) -> Result<i64, DbError> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(size_bytes), 0) as total FROM site_backups WHERE user_id = ? AND status = 'completed'",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let total: i64 = row.get("total");
        Ok(total)
    }

    /// Calculates total storage used by backups for a site.
    pub async fn get_total_backup_bytes_by_site(&self, site_id: i64) -> Result<i64, DbError> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(size_bytes), 0) as total FROM site_backups WHERE site_id = ? AND status = 'completed'",
        )
        .bind(site_id)
        .fetch_one(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let total: i64 = row.get("total");
        Ok(total)
    }

    /// Gets backup schedule configuration for a given site.
    pub async fn get_schedule(
        &self,
        site_id: i64,
    ) -> Result<Option<SiteBackupScheduleModel>, DbError> {
        let row = sqlx::query(
            r#"
            SELECT
                site_id, is_enabled, frequency, time_hour, time_minute,
                retention_count, include_files, include_databases,
                last_run_at, next_run_at, storage_destination,
                s3_endpoint, s3_bucket, s3_region, s3_access_key,
                s3_secret_key_enc, s3_prefix, updated_at
            FROM site_backup_schedules
            WHERE site_id = ?
            "#,
        )
        .bind(site_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let model = row.map(|r| SiteBackupScheduleModel {
            site_id: r.get("site_id"),
            is_enabled: r.get::<i64, _>("is_enabled") != 0,
            frequency: r.get("frequency"),
            time_hour: r.get("time_hour"),
            time_minute: r.get("time_minute"),
            retention_count: r.get("retention_count"),
            include_files: r.get::<i64, _>("include_files") != 0,
            include_databases: r.get::<i64, _>("include_databases") != 0,
            last_run_at: r.get("last_run_at"),
            next_run_at: r.get("next_run_at"),
            storage_destination: r.get("storage_destination"),
            s3_endpoint: r.get("s3_endpoint"),
            s3_bucket: r.get("s3_bucket"),
            s3_region: r.get("s3_region"),
            s3_access_key: r.get("s3_access_key"),
            s3_secret_key_enc: r.get("s3_secret_key_enc"),
            s3_prefix: r.get("s3_prefix"),
            updated_at: r.get("updated_at"),
        });

        Ok(model)
    }

    /// Upserts automated backup schedule for a site.
    pub async fn upsert_schedule(
        &self,
        input: &UpsertBackupScheduleInput,
    ) -> Result<SiteBackupScheduleModel, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let is_enabled = if input.is_enabled { 1 } else { 0 };
        let include_files = if input.include_files { 1 } else { 0 };
        let include_databases = if input.include_databases { 1 } else { 0 };

        sqlx::query(
            r#"
            INSERT INTO site_backup_schedules (
                site_id, is_enabled, frequency, time_hour, time_minute,
                retention_count, include_files, include_databases,
                storage_destination, s3_endpoint, s3_bucket, s3_region,
                s3_access_key, s3_secret_key_enc, s3_prefix, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(site_id) DO UPDATE SET
                is_enabled = excluded.is_enabled,
                frequency = excluded.frequency,
                time_hour = excluded.time_hour,
                time_minute = excluded.time_minute,
                retention_count = excluded.retention_count,
                include_files = excluded.include_files,
                include_databases = excluded.include_databases,
                storage_destination = excluded.storage_destination,
                s3_endpoint = excluded.s3_endpoint,
                s3_bucket = excluded.s3_bucket,
                s3_region = excluded.s3_region,
                s3_access_key = excluded.s3_access_key,
                s3_secret_key_enc = excluded.s3_secret_key_enc,
                s3_prefix = excluded.s3_prefix,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(input.site_id)
        .bind(is_enabled)
        .bind(&input.frequency)
        .bind(input.time_hour)
        .bind(input.time_minute)
        .bind(input.retention_count)
        .bind(include_files)
        .bind(include_databases)
        .bind(&input.storage_destination)
        .bind(&input.s3_endpoint)
        .bind(&input.s3_bucket)
        .bind(&input.s3_region)
        .bind(&input.s3_access_key)
        .bind(&input.s3_secret_key_enc)
        .bind(&input.s3_prefix)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let schedule = self
            .get_schedule(input.site_id)
            .await?
            .ok_or_else(|| DbError::NotFound("Backup schedule not found after upsert".into()))?;

        Ok(schedule)
    }

    /// Updates last run and next run timestamp of an automated schedule.
    pub async fn update_schedule_last_run(
        &self,
        site_id: i64,
        last_run_at: i64,
        next_run_at: Option<i64>,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE site_backup_schedules SET last_run_at = ?, next_run_at = ? WHERE site_id = ?",
        )
        .bind(last_run_at)
        .bind(next_run_at)
        .bind(site_id)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(())
    }

    /// Identifies completed backups that exceed retention and removes them from the database,
    /// returning the deleted records so their archive files can be safely deleted on disk.
    pub async fn prune_old_backups(
        &self,
        site_id: i64,
        retention_count: i64,
    ) -> Result<Vec<SiteBackupModel>, DbError> {
        let all_backups = self.list_backups_by_site(site_id).await?;
        let completed: Vec<SiteBackupModel> = all_backups
            .into_iter()
            .filter(|b| b.status == "completed")
            .collect();

        if completed.len() as i64 <= retention_count {
            return Ok(Vec::new());
        }

        let to_remove = &completed[retention_count as usize..];
        let mut pruned = Vec::new();

        for b in to_remove {
            self.delete_backup(b.id).await?;
            pruned.push(b.clone());
        }

        Ok(pruned)
    }
}
