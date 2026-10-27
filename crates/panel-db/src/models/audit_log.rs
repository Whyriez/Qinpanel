use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct AuditLog {
    pub id: i64,
    pub user_id: Option<i64>,
    pub actor_username: String,
    pub action: String,
    pub target_resource: String,
    pub details_json: Option<String>,
    pub ip_address: Option<String>,
    pub status: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAuditLog {
    pub user_id: Option<i64>,
    pub actor_username: String,
    pub action: String,
    pub target_resource: String,
    pub details_json: Option<String>,
    pub ip_address: Option<String>,
    pub status: String,
}
