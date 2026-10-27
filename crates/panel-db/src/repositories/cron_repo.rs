//! Cron repository for cron job management and execution history.

use crate::error::DbError;
use crate::models::{CreateCronJob, CreateCronRun, CronJob, CronRun, UpdateCronJob};
use crate::pool::DbPool;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqliteCronRepository {
    pool: DbPool,
}

impl SqliteCronRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Creates a new cron job entry in the database.
    pub async fn create_job(&self, input: &CreateCronJob) -> Result<CronJob, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let timeout_secs = input.timeout_secs.unwrap_or(3600);
        let enabled = if input.enabled.unwrap_or(true) { 1 } else { 0 };

        let res = sqlx::query(
            r#"
            INSERT INTO cron_jobs (
                site_id, name, command, schedule, systemd_calendar,
                description, work_dir, timeout_secs, enabled, status,
                created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'idle', ?, ?)
            "#,
        )
        .bind(input.site_id)
        .bind(&input.name)
        .bind(&input.command)
        .bind(&input.schedule)
        .bind(&input.systemd_calendar)
        .bind(&input.description)
        .bind(&input.work_dir)
        .bind(timeout_secs)
        .bind(enabled)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let id = res.last_insert_rowid();

        self.find_job_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Cron job {} not found after insert", id)))
    }

    /// Finds a cron job by its primary key ID.
    pub async fn find_job_by_id(&self, id: i64) -> Result<Option<CronJob>, DbError> {
        sqlx::query_as::<_, CronJob>(
            r#"
            SELECT id, site_id, name, command, schedule, systemd_calendar,
                   description, work_dir, timeout_secs, enabled, status,
                   last_run_at, last_duration_ms, last_exit_code,
                   created_at, updated_at
            FROM cron_jobs
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Lists all cron jobs configured for a specific site.
    pub async fn list_jobs_for_site(&self, site_id: i64) -> Result<Vec<CronJob>, DbError> {
        sqlx::query_as::<_, CronJob>(
            r#"
            SELECT id, site_id, name, command, schedule, systemd_calendar,
                   description, work_dir, timeout_secs, enabled, status,
                   last_run_at, last_duration_ms, last_exit_code,
                   created_at, updated_at
            FROM cron_jobs
            WHERE site_id = ?
            ORDER BY id ASC
            "#,
        )
        .bind(site_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Counts the total number of cron jobs belonging to a user (across all sites).
    pub async fn count_jobs_for_user(&self, user_id: i64) -> Result<i64, DbError> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(c.id)
            FROM cron_jobs c
            JOIN sites s ON c.site_id = s.id
            WHERE s.user_id = ?
            "#,
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        Ok(count.0)
    }

    /// Updates a cron job configuration.
    pub async fn update_job(&self, id: i64, input: &UpdateCronJob) -> Result<CronJob, DbError> {
        let existing = self
            .find_job_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Cron job {} not found", id)))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let name = input.name.as_ref().unwrap_or(&existing.name);
        let command = input.command.as_ref().unwrap_or(&existing.command);
        let schedule = input.schedule.as_ref().unwrap_or(&existing.schedule);
        let systemd_calendar = input
            .systemd_calendar
            .as_ref()
            .unwrap_or(&existing.systemd_calendar);
        let description = input.description.as_ref().or(existing.description.as_ref());
        let work_dir = input.work_dir.as_ref().or(existing.work_dir.as_ref());
        let timeout_secs = input.timeout_secs.unwrap_or(existing.timeout_secs);
        let enabled = if input.enabled.unwrap_or(existing.enabled) {
            1
        } else {
            0
        };
        let status = input.status.as_ref().unwrap_or(&existing.status);
        let last_run_at = input.last_run_at.or(existing.last_run_at);
        let last_duration_ms = input.last_duration_ms.or(existing.last_duration_ms);
        let last_exit_code = input.last_exit_code.or(existing.last_exit_code);

        sqlx::query(
            r#"
            UPDATE cron_jobs
            SET name = ?, command = ?, schedule = ?, systemd_calendar = ?,
                description = ?, work_dir = ?, timeout_secs = ?, enabled = ?,
                status = ?, last_run_at = ?, last_duration_ms = ?, last_exit_code = ?,
                updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(name)
        .bind(command)
        .bind(schedule)
        .bind(systemd_calendar)
        .bind(description)
        .bind(work_dir)
        .bind(timeout_secs)
        .bind(enabled)
        .bind(status)
        .bind(last_run_at)
        .bind(last_duration_ms)
        .bind(last_exit_code)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        self.find_job_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Cron job {} not found after update", id)))
    }

    /// Deletes a cron job by primary key ID.
    pub async fn delete_job(&self, id: i64) -> Result<bool, DbError> {
        let res = sqlx::query("DELETE FROM cron_jobs WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(DbError::Sqlx)?;

        Ok(res.rows_affected() > 0)
    }

    /// Records a new execution run for a cron job.
    pub async fn create_run(&self, input: &CreateCronRun) -> Result<CronRun, DbError> {
        let res = sqlx::query(
            r#"
            INSERT INTO cron_runs (
                cron_job_id, status, exit_code, stdout, stderr,
                duration_ms, started_at, finished_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(input.cron_job_id)
        .bind(&input.status)
        .bind(input.exit_code)
        .bind(&input.stdout)
        .bind(&input.stderr)
        .bind(input.duration_ms)
        .bind(input.started_at)
        .bind(input.finished_at)
        .execute(&self.pool)
        .await
        .map_err(DbError::Sqlx)?;

        let id = res.last_insert_rowid();

        sqlx::query_as::<_, CronRun>(
            r#"
            SELECT id, cron_job_id, status, exit_code, stdout, stderr,
                   duration_ms, started_at, finished_at
            FROM cron_runs
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }

    /// Lists recent execution runs for a cron job.
    pub async fn list_runs_for_job(
        &self,
        cron_job_id: i64,
        limit: i64,
    ) -> Result<Vec<CronRun>, DbError> {
        sqlx::query_as::<_, CronRun>(
            r#"
            SELECT id, cron_job_id, status, exit_code, stdout, stderr,
                   duration_ms, started_at, finished_at
            FROM cron_runs
            WHERE cron_job_id = ?
            ORDER BY started_at DESC
            LIMIT ?
            "#,
        )
        .bind(cron_job_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Sqlx)
    }
}
