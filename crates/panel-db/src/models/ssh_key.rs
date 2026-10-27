use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct SshKey {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub public_key: String,
    pub key_type: String,
    pub fingerprint_sha256: String,
    pub fingerprint_md5: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSshKey {
    pub user_id: i64,
    pub name: String,
    pub public_key: String,
    pub key_type: String,
    pub fingerprint_sha256: String,
    pub fingerprint_md5: Option<String>,
}
