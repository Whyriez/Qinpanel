use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct Plan {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub cpu_quota_percent: i64,
    pub memory_limit_bytes: i64,
    pub memory_high_bytes: Option<i64>,
    pub tasks_max: i64,
    pub io_weight: i64,
    pub disk_limit_bytes: i64,
    pub inode_limit: i64,
    pub max_sites: i64,
    pub max_domains: i64,
    pub max_databases: i64,
    pub max_cron_jobs: i64,
    pub max_terminal_sessions: i64,
    pub max_upload_bytes: i64,
    pub shell_policy: String,
    pub allow_external_upstream: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePlan {
    pub name: String,
    pub description: Option<String>,
    pub cpu_quota_percent: Option<i64>,
    pub memory_limit_bytes: Option<i64>,
    pub memory_high_bytes: Option<i64>,
    pub tasks_max: Option<i64>,
    pub io_weight: Option<i64>,
    pub disk_limit_bytes: Option<i64>,
    pub inode_limit: Option<i64>,
    pub max_sites: Option<i64>,
    pub max_domains: Option<i64>,
    pub max_databases: Option<i64>,
    pub max_cron_jobs: Option<i64>,
    pub max_terminal_sessions: Option<i64>,
    pub max_upload_bytes: Option<i64>,
    pub shell_policy: Option<String>,
    pub allow_external_upstream: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdatePlan {
    pub name: Option<String>,
    pub description: Option<String>,
    pub cpu_quota_percent: Option<i64>,
    pub memory_limit_bytes: Option<i64>,
    pub memory_high_bytes: Option<i64>,
    pub tasks_max: Option<i64>,
    pub io_weight: Option<i64>,
    pub disk_limit_bytes: Option<i64>,
    pub inode_limit: Option<i64>,
    pub max_sites: Option<i64>,
    pub max_domains: Option<i64>,
    pub max_databases: Option<i64>,
    pub max_cron_jobs: Option<i64>,
    pub max_terminal_sessions: Option<i64>,
    pub max_upload_bytes: Option<i64>,
    pub shell_policy: Option<String>,
    pub allow_external_upstream: Option<bool>,
}
