use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct Session {
    pub id: String,
    pub user_id: i64,
    pub token_hash: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub expires_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSession {
    pub id: String,
    pub user_id: i64,
    pub token_hash: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub expires_at: i64,
}
