pub mod handlers;

pub use handlers::*;

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SshKeyResponse {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub key_type: String,
    pub fingerprint_sha256: String,
    pub fingerprint_md5: Option<String>,
    pub public_key: String,
    pub created_at: i64,
}

impl From<panel_db::models::SshKey> for SshKeyResponse {
    fn from(k: panel_db::models::SshKey) -> Self {
        Self {
            id: k.id,
            user_id: k.user_id,
            name: k.name,
            key_type: k.key_type,
            fingerprint_sha256: k.fingerprint_sha256,
            fingerprint_md5: k.fingerprint_md5,
            public_key: k.public_key,
            created_at: k.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateSshKeyRequest {
    pub name: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ValidateSshKeyRequest {
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ValidateSshKeyResponse {
    pub valid: bool,
    pub key_type: Option<String>,
    pub fingerprint_sha256: Option<String>,
    pub fingerprint_md5: Option<String>,
    pub comment: Option<String>,
    pub bit_length: Option<usize>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct SshKeyQuery {
    pub user_id: Option<i64>,
    pub tenant: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct SshDropinApplyRequestDto {
    pub custom_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SshDropinApplyResponseDto {
    pub success: bool,
    pub path: String,
    pub validated: bool,
    pub reloaded: bool,
    pub message: String,
}

#[derive(Debug, Deserialize, IntoParams, Default)]
pub struct SshConnectionInfoQuery {
    pub site_id: Option<i64>,
    pub path: Option<String>,
    pub tenant: Option<String>,
    pub port: Option<u16>,
    pub host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SshConnectionInfoResponse {
    pub host: Option<String>,
    pub port: Option<u16>,
    /// configured | manual_preview | missing | unsupported_transport
    pub endpoint_state: String,
    pub endpoint_id: Option<String>,
    pub endpoint_revision: Option<i64>,
    pub transport: Option<String>,
    /// Local configured host identity, never a proof of remote reachability.
    pub host_keys: Vec<panel_core::ssh::SshHostKeyFingerprint>,
    pub host_keys_status: String,
    pub username: String,
    pub user_id: i64,
    pub home_dir: String,
    pub active_path: String,
    pub relative_path: String,
    pub sftp_path: String,
    pub shell_policy: String,
    pub can_shell: bool,
    pub can_sftp: bool,
    pub has_ssh_keys: bool,
    pub ssh_keys_count: usize,
    pub ssh_command: Option<String>,
    pub sftp_command: Option<String>,
    pub sftp_cd_command: Option<String>,
    pub sftp_url: Option<String>,
    pub ssh_config_snippet: Option<String>,
    pub site_domain: Option<String>,
}
