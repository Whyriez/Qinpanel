//! REST API endpoints for system notifications, unread badges, and alert settings.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, patch, post},
};
use panel_core::notification::{validate_email_address, validate_smtp_host};
use panel_db::models::{NotificationFilter, NotificationModel, UpdateNotificationSettingsInput};
use panel_db::repositories::SqliteNotificationRepository;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use crate::auth::middleware::{RequireAuth, RequireCsrf};
use crate::mail::{EmailMessage, SmtpClient, SmtpConfig};
use crate::notifications::service::NotificationService;
use crate::routes::AppState;

/// Query parameters for listing notifications.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListNotificationsQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub unread_only: Option<bool>,
    pub category: Option<String>,
    pub severity: Option<String>,
}

/// Response DTO for a single notification item.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NotificationItemResponse {
    pub id: String,
    pub user_id: Option<i64>,
    pub site_id: Option<i64>,
    pub category: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    pub metadata_json: Option<String>,
    pub action_url: Option<String>,
    pub is_read: bool,
    pub read_at: Option<i64>,
    pub created_at: i64,
}

impl From<NotificationModel> for NotificationItemResponse {
    fn from(m: NotificationModel) -> Self {
        Self {
            id: m.id,
            user_id: m.user_id,
            site_id: m.site_id,
            category: m.category,
            severity: m.severity,
            title: m.title,
            message: m.message,
            metadata_json: m.metadata_json,
            action_url: m.action_url,
            is_read: m.is_read,
            read_at: m.read_at,
            created_at: m.created_at,
        }
    }
}

/// Response DTO for paginated notification listing.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListNotificationsResponse {
    pub items: Vec<NotificationItemResponse>,
    pub total: i64,
    pub unread_count: i64,
    pub page: i64,
    pub per_page: i64,
}

/// Request DTO for testing SMTP connection.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TestSmtpRequest {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub encryption: Option<String>,
    pub to_address: Option<String>,
}

/// Request DTO for updating notification settings.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSettingsRequest {
    pub smtp_enabled: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_encryption: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub admin_email: Option<String>,
    pub ssl_warning_days: Option<i64>,
    pub quota_warning_percent: Option<i64>,
    pub notify_ssl_expiring: Option<bool>,
    pub notify_quota_warning: Option<bool>,
    pub notify_job_failed: Option<bool>,
    pub notify_deploy_failed: Option<bool>,
}

/// Query parameter for clearing notifications.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ClearNotificationsQuery {
    pub read_only: Option<bool>,
}

/// List notifications with pagination, unread filter, and categories.
async fn list_notifications(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
    Query(query): Query<ListNotificationsQuery>,
) -> impl IntoResponse {
    let notif_repo = SqliteNotificationRepository::new(state.pool.clone());
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * per_page;

    let filter = NotificationFilter {
        unread_only: query.unread_only.unwrap_or(false),
        category: query.category,
        severity: query.severity,
        limit: per_page,
        offset,
    };

    // Admin sees all notifications; regular users see their own + broadcast system notifications
    let user_filter = if user.role == "admin" {
        None
    } else {
        Some(user.id)
    };

    let (items, total) = match notif_repo.list(user_filter, &filter).await {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Gagal mengambil daftar notifikasi: {e}") })),
            )
                .into_response();
        }
    };

    let unread_count = notif_repo
        .unread_count(user_filter)
        .await
        .unwrap_or_default();

    let response_items = items
        .into_iter()
        .map(NotificationItemResponse::from)
        .collect();

    (
        StatusCode::OK,
        Json(ListNotificationsResponse {
            items: response_items,
            total,
            unread_count,
            page,
            per_page,
        }),
    )
        .into_response()
}

/// Fast unread badge count endpoint for topbar polling.
async fn get_unread_count(
    State(state): State<AppState>,
    RequireAuth(user): RequireAuth,
) -> impl IntoResponse {
    let notif_repo = SqliteNotificationRepository::new(state.pool.clone());
    let user_filter = if user.role == "admin" {
        None
    } else {
        Some(user.id)
    };

    match notif_repo.unread_count(user_filter).await {
        Ok(unread_count) => (
            StatusCode::OK,
            Json(json!({ "unread_count": unread_count })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Gagal menghitung notifikasi belum dibaca: {e}") })),
        )
            .into_response(),
    }
}

/// Mark a single notification as read.
async fn mark_notification_read(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let notif_repo = SqliteNotificationRepository::new(state.pool.clone());
    let user_filter = if user.role == "admin" {
        None
    } else {
        Some(user.id)
    };

    match notif_repo.mark_as_read(&id, user_filter).await {
        Ok(found) => {
            if !found {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": "Notifikasi tidak ditemukan" })),
                )
                    .into_response()
            } else {
                (StatusCode::OK, Json(json!({ "success": true }))).into_response()
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Gagal menandai notifikasi telah dibaca: {e}") })),
        )
            .into_response(),
    }
}

/// Mark all notifications as read for current user.
async fn mark_all_read(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
) -> impl IntoResponse {
    let notif_repo = SqliteNotificationRepository::new(state.pool.clone());
    let user_filter = if user.role == "admin" {
        None
    } else {
        Some(user.id)
    };

    match notif_repo.mark_all_read(user_filter).await {
        Ok(marked_count) => (
            StatusCode::OK,
            Json(json!({ "marked_count": marked_count })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Gagal menandai semua dibaca: {e}") })),
        )
            .into_response(),
    }
}

/// Delete a notification.
async fn delete_notification(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let notif_repo = SqliteNotificationRepository::new(state.pool.clone());
    let user_filter = if user.role == "admin" {
        None
    } else {
        Some(user.id)
    };

    match notif_repo.delete(&id, user_filter).await {
        Ok(found) => {
            if !found {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": "Notifikasi tidak ditemukan" })),
                )
                    .into_response()
            } else {
                (StatusCode::OK, Json(json!({ "success": true }))).into_response()
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Gagal menghapus notifikasi: {e}") })),
        )
            .into_response(),
    }
}

/// Clear notifications (default: only read notifications).
async fn clear_notifications(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Query(query): Query<ClearNotificationsQuery>,
) -> impl IntoResponse {
    let notif_repo = SqliteNotificationRepository::new(state.pool.clone());
    let user_filter = if user.role == "admin" {
        None
    } else {
        Some(user.id)
    };
    let read_only = query.read_only.unwrap_or(true);

    match notif_repo.clear_all(user_filter, read_only).await {
        Ok(cleared_count) => (
            StatusCode::OK,
            Json(json!({ "cleared_count": cleared_count })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Gagal membersihkan notifikasi: {e}") })),
        )
            .into_response(),
    }
}

/// Get notification and SMTP configuration settings.
async fn get_notification_settings(
    State(state): State<AppState>,
    RequireAuth(_user): RequireAuth,
) -> impl IntoResponse {
    let notif_repo = SqliteNotificationRepository::new(state.pool.clone());
    match notif_repo.get_settings().await {
        Ok(settings) => {
            // Mask password if present
            let password_masked = settings
                .smtp_password_enc
                .as_ref()
                .map(|_| "********".to_string());
            (
                StatusCode::OK,
                Json(json!({
                    "id": settings.id,
                    "smtp_enabled": settings.smtp_enabled,
                    "smtp_host": settings.smtp_host,
                    "smtp_port": settings.smtp_port,
                    "smtp_username": settings.smtp_username,
                    "smtp_password_masked": password_masked,
                    "smtp_has_password": settings.smtp_password_enc.is_some(),
                    "smtp_encryption": settings.smtp_encryption,
                    "from_address": settings.from_address,
                    "from_name": settings.from_name,
                    "admin_email": settings.admin_email,
                    "ssl_warning_days": settings.ssl_warning_days,
                    "quota_warning_percent": settings.quota_warning_percent,
                    "notify_ssl_expiring": settings.notify_ssl_expiring,
                    "notify_quota_warning": settings.notify_quota_warning,
                    "notify_job_failed": settings.notify_job_failed,
                    "notify_deploy_failed": settings.notify_deploy_failed,
                    "updated_at": settings.updated_at,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Gagal mengambil pengaturan notifikasi: {e}") })),
        )
            .into_response(),
    }
}

/// Update notification and SMTP settings (admin only).
async fn update_notification_settings(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Json(payload): Json<UpdateSettingsRequest>,
) -> impl IntoResponse {
    if user.role != "admin" {
        return (
            StatusCode::FORBIDDEN,
            Json(
                json!({ "error": "Hanya admin yang dapat mengubah pengaturan notifikasi sistem" }),
            ),
        )
            .into_response();
    }

    // Validation
    if let Some(ref host) = payload.smtp_host
        && !host.is_empty()
        && let Err(e) = validate_smtp_host(host)
    {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
    }

    if payload.smtp_port.is_some_and(|p| !(1..=65535).contains(&p)) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Port SMTP harus 1-65535"})),
        )
            .into_response();
    }
    if payload
        .smtp_encryption
        .as_ref()
        .is_some_and(|v| !["tls", "starttls", "none"].contains(&v.as_str()))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Pilihan enkripsi SMTP tidak valid"})),
        )
            .into_response();
    }
    if payload
        .from_name
        .as_ref()
        .is_some_and(|v| v.len() > 128 || v.chars().any(char::is_control))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Nama pengirim tidak valid"})),
        )
            .into_response();
    }

    if let Some(ref from) = payload.from_address
        && let Err(e) = validate_email_address(from)
    {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
    }

    if let Some(ref admin_email) = payload.admin_email
        && !admin_email.trim().is_empty()
        && let Err(e) = validate_email_address(admin_email)
    {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
    }

    if payload
        .ssl_warning_days
        .is_some_and(|days| !(1..=90).contains(&days))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Ambang batas SSL harus 1-90 hari"})),
        )
            .into_response();
    }
    if payload
        .quota_warning_percent
        .is_some_and(|pct| !(50..=99).contains(&pct))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Ambang batas kuota harus 50-99 persen"})),
        )
            .into_response();
    }
    // Don't overwrite password with masked placeholder
    let password_enc =
        match payload.smtp_password {
            Some(ref p) if p == "********" => None,
            Some(p) => match crate::email::secret(&state.agent_client, p, true).await {
                Ok(value) => Some(value),
                Err(_) => return (
                    StatusCode::BAD_GATEWAY,
                    Json(
                        json!({"error":"Password belum disimpan. Agent enkripsi tidak tersedia."}),
                    ),
                )
                    .into_response(),
            },
            None => None,
        };

    let notif_repo = SqliteNotificationRepository::new(state.pool.clone());
    let update_input = UpdateNotificationSettingsInput {
        smtp_enabled: payload.smtp_enabled,
        smtp_host: payload.smtp_host,
        smtp_port: payload.smtp_port,
        smtp_username: payload.smtp_username,
        smtp_password_enc: password_enc,
        smtp_encryption: payload.smtp_encryption,
        from_address: payload.from_address,
        from_name: payload.from_name,
        admin_email: payload.admin_email,
        ssl_warning_days: payload.ssl_warning_days,
        quota_warning_percent: payload.quota_warning_percent,
        notify_ssl_expiring: payload.notify_ssl_expiring,
        notify_quota_warning: payload.notify_quota_warning,
        notify_job_failed: payload.notify_job_failed,
        notify_deploy_failed: payload.notify_deploy_failed,
    };

    match notif_repo.update_settings(&update_input).await {
        Ok(updated) => {
            let password_masked = updated
                .smtp_password_enc
                .as_ref()
                .map(|_| "********".to_string());
            (
                StatusCode::OK,
                Json(json!({
                    "id": updated.id,
                    "smtp_enabled": updated.smtp_enabled,
                    "smtp_host": updated.smtp_host,
                    "smtp_port": updated.smtp_port,
                    "smtp_username": updated.smtp_username,
                    "smtp_password_masked": password_masked,
                    "smtp_has_password": updated.smtp_password_enc.is_some(),
                    "smtp_encryption": updated.smtp_encryption,
                    "from_address": updated.from_address,
                    "from_name": updated.from_name,
                    "admin_email": updated.admin_email,
                    "ssl_warning_days": updated.ssl_warning_days,
                    "quota_warning_percent": updated.quota_warning_percent,
                    "notify_ssl_expiring": updated.notify_ssl_expiring,
                    "notify_quota_warning": updated.notify_quota_warning,
                    "notify_job_failed": updated.notify_job_failed,
                    "notify_deploy_failed": updated.notify_deploy_failed,
                    "updated_at": updated.updated_at,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Gagal memperbarui pengaturan notifikasi: {e}") })),
        )
            .into_response(),
    }
}

/// Send a test notification email (admin only).
async fn test_smtp_connection(
    State(state): State<AppState>,
    RequireCsrf(user): RequireCsrf,
    Json(payload): Json<TestSmtpRequest>,
) -> impl IntoResponse {
    if user.role != "admin" {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Hanya admin yang dapat menguji koneksi SMTP" })),
        )
            .into_response();
    }

    let notif_repo = SqliteNotificationRepository::new(state.pool.clone());
    let current_settings = match notif_repo.get_settings().await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Gagal membaca pengaturan SMTP: {e}") })),
            )
                .into_response();
        }
    };

    let host = payload
        .host
        .or(current_settings.smtp_host)
        .unwrap_or_default();
    if host.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Host SMTP belum dikonfigurasi" })),
        )
            .into_response();
    }

    let port = payload.port.unwrap_or(current_settings.smtp_port as u16);
    let username = payload.username.or(current_settings.smtp_username);
    let password = match payload.password {
        Some(p) if p != "********" => Some(p),
        _ => match crate::email::smtp_password(
            &state.pool,
            &state.agent_client,
            current_settings.smtp_password_enc,
        )
        .await
        {
            Ok(value) => value,
            Err(_) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error":"Password SMTP tidak dapat dibuka oleh agent"})),
                )
                    .into_response();
            }
        },
    };
    let encryption = payload
        .encryption
        .unwrap_or(current_settings.smtp_encryption);
    let to_address = payload
        .to_address
        .or(current_settings.admin_email)
        .or_else(|| user.email.clone())
        .unwrap_or_else(|| "admin@localhost".to_string());

    if let Err(e) = validate_email_address(&to_address) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Alamat tujuan email uji coba tidak valid: {e}") })),
        )
            .into_response();
    }

    let smtp_config = SmtpConfig {
        host,
        port,
        username,
        password,
        encryption,
        from_address: current_settings.from_address,
        from_name: current_settings.from_name,
    };

    let test_msg = EmailMessage {
        to: to_address.clone(),
        subject: "[Qinpanel] Uji Coba Pengiriman Notifikasi Email".to_string(),
        body: "Halo!\n\nIni adalah email uji coba dari Qinpanel untuk memverifikasi pengaturan SMTP Anda.\nJika Anda menerima pesan ini, konfigurasi pengiriman email telah berfungsi dengan baik.\n\nSalam,\nTim Qinpanel".to_string(),
    };

    match SmtpClient::send_mail(&smtp_config, &test_msg).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": format!("Server SMTP menerima email uji coba untuk {}", to_address)
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "error": format!("Gagal mengirim email uji coba: {e}")
            })),
        )
            .into_response(),
    }
}

/// Manually trigger system alert evaluation (SSL & Quota).
async fn evaluate_alerts(
    State(state): State<AppState>,
    RequireCsrf(_user): RequireCsrf,
) -> impl IntoResponse {
    let service = NotificationService::new(
        state.pool.clone(),
        Some(std::sync::Arc::clone(&state.agent_client)),
    )
    .with_job_runner(std::sync::Arc::clone(&state.job_runner));
    match service.evaluate_system_alerts().await {
        Ok(triggered) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "triggered_count": triggered.len(),
                "notifications": triggered
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Gagal mengevaluasi peringatan sistem: {e}") })),
        )
            .into_response(),
    }
}

/// Builds the Axum router for system notifications.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/notifications",
            get(list_notifications).delete(clear_notifications),
        )
        .route("/api/v1/notifications/unread-count", get(get_unread_count))
        .route("/api/v1/notifications/mark-all-read", post(mark_all_read))
        .route("/api/v1/notifications/{id}", delete(delete_notification))
        .route(
            "/api/v1/notifications/{id}/read",
            patch(mark_notification_read),
        )
        .route(
            "/api/v1/notifications/settings",
            get(get_notification_settings).put(update_notification_settings),
        )
        .route(
            "/api/v1/notifications/smtp/test",
            post(test_smtp_connection),
        )
        .route("/api/v1/notifications/evaluate", post(evaluate_alerts))
}
