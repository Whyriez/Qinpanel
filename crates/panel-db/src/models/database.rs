use serde::{Deserialize, Serialize};

/// Database server daemon metadata and status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseServer {
    pub engine: String,
    pub is_installed: bool,
    pub is_running: bool,
    pub version: Option<String>,
    pub port: i64,
    pub socket_path: Option<String>,
    pub updated_at: i64,
}

/// Tenant database instance entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseModel {
    pub id: i64,
    pub user_id: i64,
    pub site_id: Option<i64>,
    pub engine: String,
    pub name: String,
    pub db_user: String,
    pub remote_access: bool,
    pub allowed_hosts: String,
    pub collation: String,
    pub character_set: String,
    pub size_bytes: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Input parameters for creating a new database record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDatabaseInput {
    pub user_id: i64,
    pub site_id: Option<i64>,
    pub engine: String,
    pub name: String,
    pub db_user: String,
    pub remote_access: bool,
    pub allowed_hosts: String,
    pub collation: String,
    pub character_set: String,
}

/// Input parameters for updating an existing database record.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateDatabaseInput {
    pub site_id: Option<Option<i64>>,
    pub remote_access: Option<bool>,
    pub allowed_hosts: Option<String>,
    pub collation: Option<String>,
    pub character_set: Option<String>,
}
