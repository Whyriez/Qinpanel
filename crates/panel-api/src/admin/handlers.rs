//! Admin management HTTP handlers protected by RBAC.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use panel_db::repositories::{SqliteSettingRepository, SqliteUserRepository};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::{MessageResponse, RequireAdmin, RequireAdminCsrf, RequireAuth};
use crate::routes::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminUserItem {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub email: Option<String>,
    pub status: String,
    pub totp_enabled: bool,
    pub linux_username: Option<String>,
    pub linux_uid: Option<i64>,
    pub plan_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUserStatusRequest {
    #[schema(example = "suspended")]
    pub status: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSettingRequest {
    #[schema(example = "panel.title")]
    pub key: String,
    #[schema(example = "My Hosting Panel")]
    pub value: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SettingResponse {
    pub key: String,
    pub value: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct SettingQuery {
    pub key: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/users",
    responses(
        (status = 200, description = "List all panel users (admin only)", body = Vec<AdminUserItem>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Administrator privileges required")
    ),
    tag = "admin"
)]
pub async fn admin_list_users_handler(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Response {
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    let users = match user_repo.list_all().await {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Database error: {}", e) })),
            )
                .into_response();
        }
    };

    let items: Vec<AdminUserItem> = users
        .into_iter()
        .map(|u| AdminUserItem {
            id: u.id,
            username: u.username,
            role: u.role,
            email: u.email,
            status: u.status,
            totp_enabled: u.totp_enabled,
            linux_username: u.linux_username,
            linux_uid: u.linux_uid,
            plan_id: u.plan_id,
            created_at: u.created_at,
            updated_at: u.updated_at,
        })
        .collect();

    Json(items).into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/users/{id}",
    params(
        ("id" = i64, Path, description = "User ID")
    ),
    responses(
        (status = 200, description = "User details (admin only)", body = AdminUserItem),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Administrator privileges required"),
        (status = 404, description = "User not found")
    ),
    tag = "admin"
)]
pub async fn admin_get_user_handler(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<i64>,
) -> Response {
    let user_repo = SqliteUserRepository::new(state.pool.clone());
    match user_repo.find_by_id(id).await {
        Ok(Some(u)) => Json(AdminUserItem {
            id: u.id,
            username: u.username,
            role: u.role,
            email: u.email,
            status: u.status,
            totp_enabled: u.totp_enabled,
            linux_username: u.linux_username,
            linux_uid: u.linux_uid,
            plan_id: u.plan_id,
            created_at: u.created_at,
            updated_at: u.updated_at,
        })
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("User ID {} not found", id) })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Database error: {}", e) })),
        )
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/users/{id}/status",
    params(
        ("id" = i64, Path, description = "User ID")
    ),
    request_body = UpdateUserStatusRequest,
    responses(
        (status = 200, description = "User status updated successfully", body = MessageResponse),
        (status = 400, description = "Invalid status value"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Administrator privileges required or invalid CSRF"),
        (status = 404, description = "User not found")
    ),
    tag = "admin"
)]
pub async fn admin_update_user_status_handler(
    State(state): State<AppState>,
    RequireAdminCsrf(admin): RequireAdminCsrf,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateUserStatusRequest>,
) -> Response {
    if payload.status != "active" && payload.status != "suspended" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Invalid status: must be 'active' or 'suspended'"
            })),
        )
            .into_response();
    }

    let user_repo = SqliteUserRepository::new(state.pool.clone());
    match user_repo.update_status(id, &payload.status).await {
        Ok(_) => {
            state.audit_writer.log_async(
                Some(id),
                &admin.username,
                "user.status_update",
                format!("user:{}", id),
                "success",
                None,
                Some(format!(r#"{{"status":"{}"}}"#, payload.status)),
            );
            Json(MessageResponse {
                message: format!("User ID {} status updated to '{}'", id, payload.status),
            })
            .into_response()
        }
        Err(panel_db::error::DbError::NotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("User ID {} not found", id) })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Database error: {}", e) })),
        )
            .into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/settings",
    params(SettingQuery),
    responses(
        (status = 200, description = "Get setting value (admin only)", body = SettingResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Administrator privileges required")
    ),
    tag = "admin"
)]
pub async fn admin_get_setting_handler(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Query(query): Query<SettingQuery>,
) -> Response {
    let setting_repo = SqliteSettingRepository::new(state.pool.clone());
    match setting_repo.get(&query.key).await {
        Ok(value) => Json(SettingResponse {
            key: query.key,
            value,
        })
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Database error: {}", e) })),
        )
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/settings",
    request_body = UpdateSettingRequest,
    responses(
        (status = 200, description = "Setting saved successfully", body = MessageResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Administrator privileges required or invalid CSRF")
    ),
    tag = "admin"
)]
pub async fn admin_update_setting_handler(
    State(state): State<AppState>,
    RequireAdminCsrf(admin): RequireAdminCsrf,
    Json(payload): Json<UpdateSettingRequest>,
) -> Response {
    let setting_repo = SqliteSettingRepository::new(state.pool.clone());
    match setting_repo.set(&payload.key, &payload.value).await {
        Ok(_) => {
            state.audit_writer.log_async(
                Some(admin.id),
                &admin.username,
                "setting.update",
                format!("setting:{}", payload.key),
                "success",
                None,
                Some(format!(
                    r#"{{"key":"{}","value":"{}"}}"#,
                    payload.key, payload.value
                )),
            );
            Json(MessageResponse {
                message: format!("Setting '{}' updated successfully", payload.key),
            })
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Database error: {}", e) })),
        )
            .into_response(),
    }
}

/// Endpoint demonstrating resource-level tenant authorization.
/// Allows a user to view their own profile, or an admin to view any user's profile.
#[utoipa::path(
    get,
    path = "/api/v1/user/{id}/profile",
    params(
        ("id" = i64, Path, description = "Target User ID")
    ),
    responses(
        (status = 200, description = "Target user profile", body = AdminUserItem),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden: Cannot access another user's profile"),
        (status = 404, description = "User not found")
    ),
    tag = "user"
)]
pub async fn user_get_profile_handler(
    State(state): State<AppState>,
    RequireAuth(auth_user): RequireAuth,
    Path(id): Path<i64>,
) -> Response {
    if !auth_user.can_access_user(id) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Forbidden: Cannot access resources belonging to another user"
            })),
        )
            .into_response();
    }

    let user_repo = SqliteUserRepository::new(state.pool.clone());
    match user_repo.find_by_id(id).await {
        Ok(Some(u)) => Json(AdminUserItem {
            id: u.id,
            username: u.username,
            role: u.role,
            email: u.email,
            status: u.status,
            totp_enabled: u.totp_enabled,
            linux_username: u.linux_username,
            linux_uid: u.linux_uid,
            plan_id: u.plan_id,
            created_at: u.created_at,
            updated_at: u.updated_at,
        })
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("User ID {} not found", id) })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Database error: {}", e) })),
        )
            .into_response(),
    }
}
