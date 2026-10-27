use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Entity representing an active or historic IP address ban with optional TTL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct SiteBannedIpModel {
    pub id: String,
    pub site_id: i64,
    pub ip_address: String,
    pub reason: String,
    pub banned_at: i64,
    pub expires_at: Option<i64>,
    pub banned_by: String,
    pub is_active: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// DTO for creating a new banned IP record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSiteBannedIpInput {
    pub id: Option<String>,
    pub site_id: i64,
    pub ip_address: String,
    pub reason: String,
    pub banned_at: i64,
    pub expires_at: Option<i64>,
    pub banned_by: String,
}
