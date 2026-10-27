use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct RecoveryCode {
    pub id: i64,
    pub user_id: i64,
    pub code_hash: String,
    pub used_at: Option<i64>,
    pub created_at: i64,
}
