use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct CronJob {
    pub id: i64,
    pub site_id: i64,
    pub name: String,
    pub command: String,
    pub schedule: String,
    pub systemd_calendar: String,
    pub description: Option<String>,
    pub work_dir: Option<String>,
    pub timeout_secs: i64,
    pub enabled: bool,
    pub status: String,
    pub last_run_at: Option<i64>,
    pub last_duration_ms: Option<i64>,
    pub last_exit_code: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCronJob {
    pub site_id: i64,
    pub name: String,
    pub command: String,
    pub schedule: String,
    pub systemd_calendar: String,
    pub description: Option<String>,
    pub work_dir: Option<String>,
    pub timeout_secs: Option<i64>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateCronJob {
    pub name: Option<String>,
    pub command: Option<String>,
    pub schedule: Option<String>,
    pub systemd_calendar: Option<String>,
    pub description: Option<String>,
    pub work_dir: Option<String>,
    pub timeout_secs: Option<i64>,
    pub enabled: Option<bool>,
    pub status: Option<String>,
    pub last_run_at: Option<i64>,
    pub last_duration_ms: Option<i64>,
    pub last_exit_code: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct CronRun {
    pub id: i64,
    pub cron_job_id: i64,
    pub status: String,
    pub exit_code: Option<i64>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub duration_ms: Option<i64>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCronRun {
    pub cron_job_id: i64,
    pub status: String,
    pub exit_code: Option<i64>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub duration_ms: Option<i64>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}
