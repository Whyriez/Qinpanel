use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct SiteGitConfig {
    pub id: i64,
    pub site_id: i64,
    pub repo_url: String,
    pub branch: String,
    pub deploy_key_pub: Option<String>,
    pub deploy_key_priv_path: Option<String>,
    pub build_command: Option<String>,
    pub post_deploy_hook: Option<String>,
    pub docroot_subpath: String,
    pub releases_to_keep: i64,
    pub webhook_secret: Option<String>,
    pub auto_deploy_enabled: bool,
    pub current_release_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertSiteGitConfig {
    pub site_id: i64,
    pub repo_url: String,
    pub branch: String,
    pub deploy_key_pub: Option<String>,
    pub deploy_key_priv_path: Option<String>,
    pub build_command: Option<String>,
    pub post_deploy_hook: Option<String>,
    pub docroot_subpath: Option<String>,
    pub releases_to_keep: Option<i64>,
    pub webhook_secret: Option<String>,
    pub auto_deploy_enabled: Option<bool>,
    pub current_release_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct SiteDeployment {
    pub id: i64,
    pub site_id: i64,
    pub release_id: String,
    pub commit_hash: Option<String>,
    pub commit_message: Option<String>,
    pub commit_author: Option<String>,
    pub branch: String,
    pub trigger_type: String,
    pub status: String,
    pub log_output: Option<String>,
    pub duration_ms: Option<i64>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSiteDeployment {
    pub site_id: i64,
    pub release_id: String,
    pub commit_hash: Option<String>,
    pub commit_message: Option<String>,
    pub commit_author: Option<String>,
    pub branch: String,
    pub trigger_type: String,
    pub status: String,
    pub log_output: Option<String>,
    pub duration_ms: Option<i64>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateSiteDeployment {
    pub status: String,
    pub commit_hash: Option<String>,
    pub commit_message: Option<String>,
    pub commit_author: Option<String>,
    pub log_output: Option<String>,
    pub duration_ms: Option<i64>,
    pub finished_at: Option<i64>,
}

/// Deploy marker used by Phase 9 Analytics timeline correlation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct DeployMarker {
    pub id: i64,
    pub site_id: i64,
    pub release_id: String,
    pub commit_hash: Option<String>,
    pub branch: String,
    pub commit_message: Option<String>,
    pub commit_author: Option<String>,
    pub trigger_type: String,
    pub created_at: i64,
}
