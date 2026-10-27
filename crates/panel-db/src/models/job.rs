use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct JobEvent {
    pub sequence: i64,
    pub job_id: String,
    pub status: String,
    pub progress: i64,
    pub message: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct Job {
    pub id: String,
    pub user_id: Option<i64>,
    pub name: String,
    pub status: String,
    pub progress: i64,
    pub payload_json: String,
    pub result_json: Option<String>,
    pub error_message: Option<String>,
    pub retry_count: i64,
    pub max_retries: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateJob {
    pub id: String,
    pub user_id: Option<i64>,
    pub name: String,
    pub payload_json: String,
    pub max_retries: i64,
}

/// Observation of a separately supervised worker, never an executable request.
#[derive(Clone)]
pub struct ExternalJobSnapshot {
    pub id: String,
    pub user_id: Option<i64>,
    pub name: String,
    pub status: String,
    pub progress: i64,
    pub payload_json: String,
    pub result_json: String,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}
