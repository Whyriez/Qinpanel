//! Plan repository for hosting plans and cgroup slice limits management.

use crate::error::DbError;
use crate::models::{CreatePlan, Plan, UpdatePlan};
use crate::pool::DbPool;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SqlitePlanRepository {
    pool: DbPool,
}

impl SqlitePlanRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, input: &CreatePlan) -> Result<Plan, DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let cpu_quota = input.cpu_quota_percent.unwrap_or(100);
        let mem_limit = input.memory_limit_bytes.unwrap_or(1073741824);
        let tasks_max = input.tasks_max.unwrap_or(100);
        let io_weight = input.io_weight.unwrap_or(100);
        let disk_limit = input.disk_limit_bytes.unwrap_or(10737418240);
        let inode_limit = input.inode_limit.unwrap_or(500000);
        let max_sites = input.max_sites.unwrap_or(5);
        let max_domains = input.max_domains.unwrap_or(10);
        let max_dbs = input.max_databases.unwrap_or(5);
        let max_cron = input.max_cron_jobs.unwrap_or(10);
        let max_terms = input.max_terminal_sessions.unwrap_or(2);
        let max_upload = input.max_upload_bytes.unwrap_or(104857600);
        let shell_policy = input.shell_policy.as_deref().unwrap_or("none");
        let allow_upstream = input.allow_external_upstream.unwrap_or(false);

        let res = sqlx::query(
            r#"
            INSERT INTO plans (
                name, description, cpu_quota_percent, memory_limit_bytes, memory_high_bytes,
                tasks_max, io_weight, disk_limit_bytes, inode_limit, max_sites, max_domains,
                max_databases, max_cron_jobs, max_terminal_sessions, max_upload_bytes,
                shell_policy, allow_external_upstream, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&input.name)
        .bind(&input.description)
        .bind(cpu_quota)
        .bind(mem_limit)
        .bind(input.memory_high_bytes)
        .bind(tasks_max)
        .bind(io_weight)
        .bind(disk_limit)
        .bind(inode_limit)
        .bind(max_sites)
        .bind(max_domains)
        .bind(max_dbs)
        .bind(max_cron)
        .bind(max_terms)
        .bind(max_upload)
        .bind(shell_policy)
        .bind(if allow_upstream { 1 } else { 0 })
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(ref db_err) if db_err.is_unique_violation() => {
                DbError::Conflict(format!("Plan name '{}' already exists", input.name))
            }
            other => DbError::Sqlx(other),
        })?;

        let id = res.last_insert_rowid();
        self.find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Plan {} not found after insert", id)))
    }

    pub async fn find_by_id(&self, id: i64) -> Result<Option<Plan>, DbError> {
        let plan = sqlx::query_as::<_, Plan>(
            r#"
            SELECT
                id, name, description, cpu_quota_percent, memory_limit_bytes, memory_high_bytes,
                tasks_max, io_weight, disk_limit_bytes, inode_limit, max_sites, max_domains,
                max_databases, max_cron_jobs, max_terminal_sessions, max_upload_bytes,
                shell_policy, allow_external_upstream, created_at, updated_at
            FROM plans
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(plan)
    }

    pub async fn find_by_name(&self, name: &str) -> Result<Option<Plan>, DbError> {
        let plan = sqlx::query_as::<_, Plan>(
            r#"
            SELECT
                id, name, description, cpu_quota_percent, memory_limit_bytes, memory_high_bytes,
                tasks_max, io_weight, disk_limit_bytes, inode_limit, max_sites, max_domains,
                max_databases, max_cron_jobs, max_terminal_sessions, max_upload_bytes,
                shell_policy, allow_external_upstream, created_at, updated_at
            FROM plans
            WHERE name = ?
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(plan)
    }

    pub async fn list_all(&self) -> Result<Vec<Plan>, DbError> {
        let plans = sqlx::query_as::<_, Plan>(
            r#"
            SELECT
                id, name, description, cpu_quota_percent, memory_limit_bytes, memory_high_bytes,
                tasks_max, io_weight, disk_limit_bytes, inode_limit, max_sites, max_domains,
                max_databases, max_cron_jobs, max_terminal_sessions, max_upload_bytes,
                shell_policy, allow_external_upstream, created_at, updated_at
            FROM plans
            ORDER BY id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(plans)
    }

    pub async fn update(&self, id: i64, input: &UpdatePlan) -> Result<Plan, DbError> {
        let current = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Plan {} not found", id)))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let name = input.name.as_deref().unwrap_or(&current.name);
        let description = input.description.as_ref().or(current.description.as_ref());
        let cpu_quota = input.cpu_quota_percent.unwrap_or(current.cpu_quota_percent);
        let mem_limit = input
            .memory_limit_bytes
            .unwrap_or(current.memory_limit_bytes);
        let mem_high = input.memory_high_bytes.or(current.memory_high_bytes);
        let tasks_max = input.tasks_max.unwrap_or(current.tasks_max);
        let io_weight = input.io_weight.unwrap_or(current.io_weight);
        let disk_limit = input.disk_limit_bytes.unwrap_or(current.disk_limit_bytes);
        let inode_limit = input.inode_limit.unwrap_or(current.inode_limit);
        let max_sites = input.max_sites.unwrap_or(current.max_sites);
        let max_domains = input.max_domains.unwrap_or(current.max_domains);
        let max_dbs = input.max_databases.unwrap_or(current.max_databases);
        let max_cron = input.max_cron_jobs.unwrap_or(current.max_cron_jobs);
        let max_terms = input
            .max_terminal_sessions
            .unwrap_or(current.max_terminal_sessions);
        let max_upload = input.max_upload_bytes.unwrap_or(current.max_upload_bytes);
        let shell_policy = input
            .shell_policy
            .as_deref()
            .unwrap_or(&current.shell_policy);
        let allow_upstream = input
            .allow_external_upstream
            .unwrap_or(current.allow_external_upstream);

        sqlx::query(
            r#"
            UPDATE plans SET
                name = ?, description = ?, cpu_quota_percent = ?, memory_limit_bytes = ?,
                memory_high_bytes = ?, tasks_max = ?, io_weight = ?, disk_limit_bytes = ?,
                inode_limit = ?, max_sites = ?, max_domains = ?, max_databases = ?,
                max_cron_jobs = ?, max_terminal_sessions = ?, max_upload_bytes = ?,
                shell_policy = ?, allow_external_upstream = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(name)
        .bind(description)
        .bind(cpu_quota)
        .bind(mem_limit)
        .bind(mem_high)
        .bind(tasks_max)
        .bind(io_weight)
        .bind(disk_limit)
        .bind(inode_limit)
        .bind(max_sites)
        .bind(max_domains)
        .bind(max_dbs)
        .bind(max_cron)
        .bind(max_terms)
        .bind(max_upload)
        .bind(shell_policy)
        .bind(if allow_upstream { 1 } else { 0 })
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(ref db_err) if db_err.is_unique_violation() => {
                DbError::Conflict(format!("Plan name '{}' already exists", name))
            }
            other => DbError::Sqlx(other),
        })?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Plan {} not found after update", id)))
    }

    pub async fn delete(&self, id: i64) -> Result<(), DbError> {
        // Verify no user is assigned to this plan
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE plan_id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;

        if count > 0 {
            return Err(DbError::Conflict(format!(
                "Cannot delete plan {}: currently assigned to {} tenant user(s)",
                id, count
            )));
        }

        let res = sqlx::query("DELETE FROM plans WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if res.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Plan with id {} not found", id)));
        }
        Ok(())
    }
}
