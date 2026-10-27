//! Retired public bootstrap routes. Administrator creation is a local CLI operation.
use axum::{Json, http::StatusCode};
use serde::{Deserialize, Serialize};
#[derive(Deserialize, utoipa::ToSchema)]
pub struct SetupTokenRequest {
    pub token: String,
}
#[derive(Deserialize, utoipa::ToSchema)]
pub struct SetupRequest {
    pub token: String,
    pub username: String,
    pub email: Option<String>,
    pub password: String,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct SetupResponse {
    pub user_id: i64,
    pub username: String,
    pub created: bool,
}

fn retired() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::GONE,
        Json(
            serde_json::json!({"error": "Setup token sudah dihentikan. Gunakan akun dari installer atau jalankan sudo panelctl admin bootstrap melalui SSH."}),
        ),
    )
}
#[utoipa::path(post, path="/api/v1/auth/setup/validate", responses((status=410,description="Public token setup retired")), tag="auth")]
pub async fn validate_token() -> (StatusCode, Json<serde_json::Value>) {
    retired()
}
#[utoipa::path(post, path="/api/v1/auth/setup", responses((status=410,description="Use local admin bootstrap")), tag="auth")]
pub async fn create_administrator() -> (StatusCode, Json<serde_json::Value>) {
    retired()
}
