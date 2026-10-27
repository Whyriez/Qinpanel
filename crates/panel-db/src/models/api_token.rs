use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct ApiToken {
    pub id: String,
    pub user_id: i64,
    pub name: String,
    pub token_hash: String,
    pub scopes_json: String,
    pub expires_at: Option<i64>,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateApiToken {
    pub id: String,
    pub user_id: i64,
    pub name: String,
    pub token_hash: String,
    pub scopes_json: String,
    pub expires_at: Option<i64>,
}
