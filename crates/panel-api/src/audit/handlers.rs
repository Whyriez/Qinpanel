//! HTTP handlers for audit log query and retrieval with RBAC enforcement.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use panel_core::audit::AuditCategory;
use panel_core::rbac::Action;
use panel_db::models::AuditLog;
use panel_db::repositories::{AuditLogFilter, SqliteAuditLogRepository};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::RequireAuth;
use crate::routes::AppState;

#[derive(Debug, Deserialize, IntoParams)]
pub struct AuditLogQuery {
    pub category: Option<AuditCategory>,
    /// Literal search across action, actor, target and IP (maximum 200 characters).
    pub q: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub user_id: Option<i64>,
    pub actor: Option<String>,
    pub action: Option<String>,
    pub target: Option<String>,
    pub status: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuditLogItem {
    pub category: AuditCategory,
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

impl From<AuditLog> for AuditLogItem {
    fn from(log: AuditLog) -> Self {
        Self {
            category: AuditCategory::for_action(&log.action),
            id: log.id,
            user_id: log.user_id,
            actor_username: log.actor_username,
            action: log.action,
            target_resource: log.target_resource,
            details_json: log.details_json,
            ip_address: log.ip_address,
            status: log.status,
            created_at: log.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuditLogListResponse {
    pub items: Vec<AuditLogItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
}

#[utoipa::path(
    get,
    path = "/api/v1/audit/logs",
    params(AuditLogQuery),
    responses(
        (status = 200, description = "Paginated list of audit logs", body = AuditLogListResponse),
        (status = 400, description = "Invalid category or search query"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden: Insufficient permissions to view these audit logs")
    ),
    tag = "audit"
)]
pub async fn list_audit_logs_handler(
    State(state): State<AppState>,
    RequireAuth(auth_user): RequireAuth,
    Query(query): Query<AuditLogQuery>,
) -> Response {
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1).saturating_mul(per_page);
    let q = query
        .q
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if q.as_ref().is_some_and(|value| value.chars().count() > 200) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Pencarian maksimal 200 karakter."})),
        )
            .into_response();
    }

    // RBAC policy evaluation:
    // If user is admin: can view all logs (target_user_id: None) or any specific user.
    // If user is regular tenant: can ONLY view their own audit logs (target_user_id: Some(auth_user.id)).
    let target_user_id = if auth_user.is_admin() {
        query.user_id
    } else {
        if query.user_id.is_some() && query.user_id != Some(auth_user.id) {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "Forbidden: Cannot view audit logs of another user"
                })),
            )
                .into_response();
        }
        Some(auth_user.id)
    };

    let filter = AuditLogFilter {
        category: query.category,
        q,
        user_id: target_user_id,
        actor_username: query.actor,
        action: query.action,
        target_resource: query.target,
        status: query.status,
        from_time: query.from,
        to_time: query.to,
    };

    let repo = SqliteAuditLogRepository::new(state.operations_read_pool.clone());
    let total = match repo.count_filtered(&filter).await {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Database error: {}", e) })),
            )
                .into_response();
        }
    };

    let logs = match repo.list_filtered(&filter, per_page, offset).await {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Database error: {}", e) })),
            )
                .into_response();
        }
    };

    let items: Vec<AuditLogItem> = logs.into_iter().map(AuditLogItem::from).collect();
    let total_pages = if total == 0 {
        0
    } else {
        (total + per_page - 1) / per_page
    };

    Json(AuditLogListResponse {
        items,
        total,
        page,
        per_page,
        total_pages,
    })
    .into_response()
}

#[utoipa::path(
    get,
    path = "/api/v1/audit/logs/{id}",
    params(
        ("id" = i64, Path, description = "Audit Log ID")
    ),
    responses(
        (status = 200, description = "Audit log detail", body = AuditLogItem),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden: Cannot access this audit log"),
        (status = 404, description = "Audit log not found")
    ),
    tag = "audit"
)]
pub async fn get_audit_log_handler(
    State(state): State<AppState>,
    RequireAuth(auth_user): RequireAuth,
    Path(id): Path<i64>,
) -> Response {
    let repo = SqliteAuditLogRepository::new(state.operations_read_pool.clone());
    let log = match repo.find_by_id(id).await {
        Ok(Some(l)) => l,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Audit log not found" })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Database error: {}", e) })),
            )
                .into_response();
        }
    };

    // RBAC policy check:
    // Admin can view any log. Regular user can only view logs where user_id == Some(auth_user.id).
    if !auth_user.authorize(&Action::ViewAuditLogs {
        target_user_id: log.user_id,
    }) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Forbidden: Cannot access audit logs of other users"
            })),
        )
            .into_response();
    }

    Json(AuditLogItem::from(log)).into_response()
}
