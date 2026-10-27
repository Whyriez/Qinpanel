use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub email: Option<String>,
    pub totp_secret: Option<String>,
    pub totp_enabled: bool,
    pub status: String,
    pub linux_username: Option<String>,
    pub linux_uid: Option<i64>,
    pub plan_id: Option<i64>,
    pub shell_policy: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUser {
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub email: Option<String>,
    pub linux_username: Option<String>,
    pub linux_uid: Option<i64>,
    pub plan_id: Option<i64>,
}
