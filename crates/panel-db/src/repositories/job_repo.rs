//! Job repository for background tasks and asynchronous operations.

use crate::error::DbError;
use crate::models::{CreateJob, Job};
use crate::pool::DbPool;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqliteJobRepository {
    pool: DbPool,
}

impl SqliteJobRepository {
    /// Only a status observer may update these entries. Workers never claim them.
    pub async fn observe_server_backup(
        &self,
        value: &crate::models::ExternalJobSnapshot,
    ) -> Result<Option<Job>, DbError> {
        if !matches!(
            value.name.as_str(),
            "server.backup.create" | "server.backup.inspect" | "server.backup.restore"
        ) || !matches!(
            value.status.as_str(),
            "queued" | "running" | "succeeded" | "failed"
        ) || !(0..=100).contains(&value.progress)
            || value.payload_json.len() > 4096
            || value.result_json.len() > 8192
        {
            return Err(DbError::Conflict(
                "Invalid external backup observation".into(),
            ));
        }
        let changed=sqlx::query_as::<_,Job>(r#"
            INSERT INTO jobs(id,user_id,name,status,progress,payload_json,result_json,error_message,retry_count,max_retries,created_at,updated_at)
            VALUES(?,?,?,?,?,?,?,?,0,0,?,?)
            ON CONFLICT(id) DO UPDATE SET user_id=COALESCE(jobs.user_id,excluded.user_id),status=excluded.status,
                progress=MAX(jobs.progress,excluded.progress),result_json=excluded.result_json,error_message=excluded.error_message,
                updated_at=MAX(jobs.updated_at,excluded.updated_at)
            WHERE jobs.name=excluded.name AND jobs.payload_json=excluded.payload_json
                AND (jobs.status NOT IN ('succeeded','failed','cancelled') OR jobs.status=excluded.status)
                AND NOT (jobs.status='running' AND excluded.status='queued')
                AND (excluded.status IN ('succeeded','failed') OR excluded.progress>=jobs.progress)
                AND (excluded.updated_at>=jobs.updated_at OR excluded.status IN ('succeeded','failed'))
                AND (jobs.status<>excluded.status OR jobs.progress<excluded.progress OR jobs.result_json IS NOT excluded.result_json
                     OR jobs.error_message IS NOT excluded.error_message OR (jobs.user_id IS NULL AND excluded.user_id IS NOT NULL))
            RETURNING id,user_id,name,status,progress,payload_json,result_json,error_message,retry_count,max_retries,created_at,updated_at
        "#).bind(&value.id).bind(value.user_id).bind(&value.name).bind(&value.status).bind(value.progress)
            .bind(&value.payload_json).bind(&value.result_json).bind(&value.error_message).bind(value.created_at).bind(value.updated_at)
            .fetch_optional(&self.pool).await?;
        if changed.is_none()
            && let Some(old) = self.find_by_id(&value.id).await?
            && (old.name != value.name || old.payload_json != value.payload_json)
        {
            return Err(DbError::Conflict(
                "Job identity belongs to another operation".into(),
            ));
        }
        Ok(changed)
    }
    pub async fn active_server_backups(&self) -> Result<Vec<Job>, DbError> {
        Ok(sqlx::query_as::<_,Job>("SELECT * FROM jobs WHERE name IN ('server.backup.create','server.backup.inspect','server.backup.restore') AND status IN ('queued','running') ORDER BY created_at LIMIT 100").fetch_all(&self.pool).await?)
    }
    /// SQLite serializes this INSERT/SELECT transaction across API workers.
    pub async fn create_or_find_active(&self, input: &CreateJob) -> Result<(Job, bool), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query(
            "INSERT INTO jobs (id,user_id,name,status,progress,payload_json,retry_count,max_retries,created_at,updated_at) SELECT ?,?,?,'queued',0,?,0,?,?,? WHERE NOT EXISTS (SELECT 1 FROM jobs WHERE name=? AND payload_json=? AND status IN ('queued','running'))"
        ).bind(&input.id).bind(input.user_id).bind(&input.name).bind(&input.payload_json)
            .bind(input.max_retries).bind(now).bind(now).bind(&input.name).bind(&input.payload_json)
            .execute(&mut *tx).await?.rows_affected() > 0;
        let job = sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE name=? AND payload_json=? AND status IN ('queued','running') ORDER BY created_at,id LIMIT 1")
            .bind(&input.name).bind(&input.payload_json).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok((job, !inserted))
    }

    pub async fn append_event(&self, event: &crate::models::job::JobEvent) -> Result<i64, DbError> {
        let mut tx = self.pool.begin().await?;
        let sequence = sqlx::query(
            "INSERT INTO job_events(job_id,status,progress,message,timestamp) VALUES(?,?,?,?,?)",
        )
        .bind(&event.job_id)
        .bind(&event.status)
        .bind(event.progress)
        .bind(&event.message)
        .bind(event.timestamp)
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();
        // Bound retained history independently per job; live delivery continues.
        sqlx::query("DELETE FROM job_events WHERE job_id=? AND sequence <= (SELECT sequence FROM job_events WHERE job_id=? ORDER BY sequence DESC LIMIT 1 OFFSET 5000)")
            .bind(&event.job_id).bind(&event.job_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(sequence)
    }

    pub async fn events_after(
        &self,
        job_id: &str,
        after: i64,
    ) -> Result<Vec<crate::models::job::JobEvent>, DbError> {
        Ok(sqlx::query_as("SELECT sequence,job_id,status,progress,message,timestamp FROM job_events WHERE job_id=? AND sequence>? ORDER BY sequence LIMIT 5000")
            .bind(job_id).bind(after).fetch_all(&self.pool).await?)
    }
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, input: &CreateJob) -> Result<Job, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        sqlx::query(
            r#"
            INSERT INTO jobs (
                id, user_id, name, status, progress, payload_json,
                result_json, error_message, retry_count, max_retries, created_at, updated_at
            )
            VALUES (?, ?, ?, 'queued', 0, ?, NULL, NULL, 0, ?, ?, ?)
            "#,
        )
        .bind(&input.id)
        .bind(input.user_id)
        .bind(&input.name)
        .bind(&input.payload_json)
        .bind(input.max_retries)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(Job {
            id: input.id.clone(),
            user_id: input.user_id,
            name: input.name.clone(),
            status: "queued".to_string(),
            progress: 0,
            payload_json: input.payload_json.clone(),
            result_json: None,
            error_message: None,
            retry_count: 0,
            max_retries: input.max_retries,
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn find_by_id(&self, id: &str) -> Result<Option<Job>, DbError> {
        let job = sqlx::query_as::<_, Job>(
            r#"
            SELECT
                id, user_id, name, status, progress, payload_json,
                result_json, error_message, retry_count, max_retries, created_at, updated_at
            FROM jobs
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(job)
    }

    pub async fn update_progress(&self, id: &str, progress: i64) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let res = sqlx::query("UPDATE jobs SET progress = ?, updated_at = ? WHERE id = ?")
            .bind(progress)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;

        if res.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Job {} not found", id)));
        }
        Ok(())
    }

    pub async fn update_status(
        &self,
        id: &str,
        status: &str,
        result_json: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let res = sqlx::query(
            r#"
            UPDATE jobs
            SET status = ?, result_json = ?, error_message = ?, updated_at = ?,
                progress = CASE WHEN ? = 'succeeded' THEN 100 ELSE progress END
            WHERE id = ?
            "#,
        )
        .bind(status)
        .bind(result_json)
        .bind(error_message)
        .bind(now)
        .bind(status)
        .bind(id)
        .execute(&self.pool)
        .await?;

        if res.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Job {} not found", id)));
        }
        Ok(())
    }

    /// Atomically claims the oldest queued job and clears the previous attempt's error.
    pub async fn claim_next_queued(&self) -> Result<Option<Job>, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let job = sqlx::query_as::<_, Job>(
            r#"
            UPDATE jobs
            SET status = 'running', error_message = NULL, updated_at = ?
            WHERE id = (
                SELECT id FROM jobs
                WHERE status = 'queued'
                  AND name NOT IN ('server.backup.create','server.backup.inspect','server.backup.restore')
                ORDER BY created_at ASC, id ASC
                LIMIT 1
            )
            RETURNING id, user_id, name, status, progress, payload_json,
                      result_json, error_message, retry_count, max_retries, created_at, updated_at
            "#,
        )
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        Ok(job)
    }

    /// Cancels a job if it is currently 'queued' or 'running'.
    pub async fn cancel(&self, id: &str) -> Result<bool, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let res = sqlx::query(
            r#"
            UPDATE jobs
            SET status = 'cancelled', updated_at = ?
            WHERE id = ? AND status IN ('queued', 'running')
              AND name NOT IN ('server.backup.create','server.backup.inspect','server.backup.restore')
              AND NOT (status = 'running' AND name IN ('php.install','php.repo.setup','database.server.install','database.web.install','cloudflare.install','cloudflare.change','email.install','email.mailbox','email.send','email.deliver','panel.tls.issue','ssl.dns01.issue','storage.tls.issue'))
            "#,
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() > 0)
    }

    /// Increments retry count. If retries remain, requeues the job; otherwise marks it failed.
    pub async fn increment_retry_or_fail(
        &self,
        id: &str,
        error_message: &str,
    ) -> Result<Job, DbError> {
        let job = match self.find_by_id(id).await? {
            Some(j) => j,
            None => return Err(DbError::NotFound(format!("Job {} not found", id))),
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let new_retry = job.retry_count + 1;
        let new_status = if new_retry < job.max_retries {
            "queued"
        } else {
            "failed"
        };

        sqlx::query(
            r#"
            UPDATE jobs
            SET status = ?, retry_count = ?, error_message = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(new_status)
        .bind(new_retry)
        .bind(error_message)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        let mut updated = job;
        updated.status = new_status.to_string();
        updated.retry_count = new_retry;
        updated.error_message = Some(error_message.to_string());
        updated.updated_at = now;
        Ok(updated)
    }

    pub async fn list_by_status(&self, status: &str) -> Result<Vec<Job>, DbError> {
        let jobs = sqlx::query_as::<_, Job>(
            r#"
            SELECT
                id, user_id, name, status, progress, payload_json,
                result_json, error_message, retry_count, max_retries, created_at, updated_at
            FROM jobs
            WHERE status = ?
            ORDER BY created_at ASC
            "#,
        )
        .bind(status)
        .fetch_all(&self.pool)
        .await?;

        Ok(jobs)
    }

    pub async fn list_by_user(&self, user_id: i64, limit: i64) -> Result<Vec<Job>, DbError> {
        self.list_paginated(Some(user_id), None, limit, 0).await
    }

    pub async fn list_paginated(
        &self,
        user_id: Option<i64>,
        status: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Job>, DbError> {
        self.list_paginated_observing(user_id, status, limit, offset, None)
            .await
    }

    pub async fn list_paginated_observing(
        &self,
        user_id: Option<i64>,
        status: Option<&str>,
        limit: i64,
        offset: i64,
        observation: Option<&crate::models::ExternalJobSnapshot>,
    ) -> Result<Vec<Job>, DbError> {
        let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT id, user_id, name, status, progress, payload_json, result_json, error_message, retry_count, max_retries, created_at, updated_at FROM jobs WHERE 1=1",
        );

        if let Some(uid) = user_id {
            builder.push(" AND user_id = ");
            builder.push_bind(uid);
        }
        Self::observed_status_filter(&mut builder, status, observation);

        builder.push(" ORDER BY created_at DESC, id DESC LIMIT ");
        builder.push_bind(limit);
        builder.push(" OFFSET ");
        builder.push_bind(offset);

        let query = builder.build_query_as::<Job>();
        let jobs = query.fetch_all(&self.pool).await?;
        Ok(jobs)
    }

    pub async fn count_filtered(
        &self,
        user_id: Option<i64>,
        status: Option<&str>,
    ) -> Result<i64, DbError> {
        self.count_filtered_observing(user_id, status, None).await
    }

    pub async fn count_filtered_observing(
        &self,
        user_id: Option<i64>,
        status: Option<&str>,
        observation: Option<&crate::models::ExternalJobSnapshot>,
    ) -> Result<i64, DbError> {
        let mut builder =
            sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT COUNT(*) FROM jobs WHERE 1=1");

        if let Some(uid) = user_id {
            builder.push(" AND user_id = ");
            builder.push_bind(uid);
        }
        Self::observed_status_filter(&mut builder, status, observation);

        let query = builder.build_query_scalar::<i64>();
        let count = query.fetch_one(&self.pool).await?;
        Ok(count)
    }

    fn observed_status_filter<'a>(
        builder: &mut sqlx::QueryBuilder<'a, sqlx::Sqlite>,
        status: Option<&'a str>,
        observation: Option<&'a crate::models::ExternalJobSnapshot>,
    ) {
        if let Some(status) = status {
            if let Some(value) = observation {
                builder
                    .push(" AND CASE WHEN id=")
                    .push_bind(&value.id)
                    .push(" AND name=")
                    .push_bind(&value.name)
                    .push(" AND status IN ('queued','running') THEN ")
                    .push_bind(&value.status)
                    .push(" ELSE status END = ")
                    .push_bind(status);
            } else {
                builder.push(" AND status = ").push_bind(status);
            }
        }
    }

    /// Recovers jobs left in 'running' state after an unexpected server restart.
    pub async fn recover_interrupted_jobs(&self) -> Result<u64, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let res = sqlx::query(
            r#"
            UPDATE jobs
            SET status = 'failed', error_message = 'Interrupted by server restart', updated_at = ?
            WHERE status = 'running'
              AND name NOT IN ('server.backup.create','server.backup.inspect','server.backup.restore')
            "#,
        )
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected())
    }
}
