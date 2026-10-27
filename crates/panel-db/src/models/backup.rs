use serde::{Deserialize, Serialize};

/// Entity representing a site backup archive record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteBackupModel {
    pub id: i64,
    pub site_id: i64,
    pub user_id: i64,
    pub filename: String,
    pub filepath: String,
    pub size_bytes: i64,
    pub label: Option<String>,
    pub includes_files: bool,
    pub includes_databases: bool,
    pub status: String,
    pub error_message: Option<String>,
    pub storage_type: String,
    pub s3_key: Option<String>,
    pub checksum_sha256: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// DTO for inserting a new site backup record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBackupInput {
    pub site_id: i64,
    pub user_id: i64,
    pub filename: String,
    pub filepath: String,
    pub size_bytes: i64,
    pub label: Option<String>,
    pub includes_files: bool,
    pub includes_databases: bool,
    pub status: String,
    pub storage_type: String,
    pub s3_key: Option<String>,
    pub checksum_sha256: Option<String>,
}

/// Entity representing the automated backup schedule configuration for a site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteBackupScheduleModel {
    pub site_id: i64,
    pub is_enabled: bool,
    pub frequency: String,
    pub time_hour: i64,
    pub time_minute: i64,
    pub retention_count: i64,
    pub include_files: bool,
    pub include_databases: bool,
    pub last_run_at: Option<i64>,
    pub next_run_at: Option<i64>,
    pub storage_destination: String,
    pub s3_endpoint: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_region: Option<String>,
    pub s3_access_key: Option<String>,
    pub s3_secret_key_enc: Option<String>,
    pub s3_prefix: Option<String>,
    pub updated_at: i64,
}

/// DTO for creating or updating automated backup schedule for a site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertBackupScheduleInput {
    pub site_id: i64,
    pub is_enabled: bool,
    pub frequency: String,
    pub time_hour: i64,
    pub time_minute: i64,
    pub retention_count: i64,
    pub include_files: bool,
    pub include_databases: bool,
    pub storage_destination: String,
    pub s3_endpoint: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_region: Option<String>,
    pub s3_access_key: Option<String>,
    pub s3_secret_key_enc: Option<String>,
    pub s3_prefix: Option<String>,
}
