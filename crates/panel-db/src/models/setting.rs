use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct Setting {
    pub key: String,
    pub value: String,
    pub updated_at: i64,
}
