//! HTTP REST API handlers for terminal session management.
//!
//! Complies with PRD Section 5.4 and Task P6-T2:
//! - Session creation with tenant quota and plan enforcement
//! - Directory confinement within tenant home
//! - Session listing, inspection, resizing, and shutdown
//! - Audit trail logging for session lifecycle events

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use panel_db::repositories::SqlitePlanRepository;
use tracing::{info, warn};

use crate::auth::AuthenticatedUser;
use crate::routes::AppState;
use crate::terminal::{
    CreateTerminalSessionRequest, TerminalAuditItem, TerminalAuditListResponse, TerminalAuditQuery,
    TerminalCloseApiResponse, TerminalListQuery, TerminalResizeRequest, TerminalResizeResponse,
    TerminalSessionListResponse, TerminalSessionResponse,
};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 900; // 15 minutes

async fn resolve_terminal_user(
    state: &AppState,
    user: &AuthenticatedUser,
    target_tenant: Option<&str>,
) -> Result<(i64, String, u32, u32, usize), (StatusCode, Json<serde_json::Value>)> {
    let context =
        crate::tenant_context::resolve_tenant_context(state, user, target_tenant, None).await?;
    let plan = if let Some(id) = context.user.plan_id {
        SqlitePlanRepository::new(state.pool.clone())
            .find_by_id(id)
            .await
            .map_err(|err| {
                tracing::error!(error=%err, "Terminal plan lookup failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error":"Gagal memuat kebijakan terminal."})),
                )
            })?
    } else {
        None
    };
    let policy = context
        .user
        .shell_policy
        .as_deref()
        .or_else(|| plan.as_ref().map(|p| p.shell_policy.as_str()))
        .unwrap_or("none");
    if policy != "full" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(
                serde_json::json!({"error":"Kebijakan shell tenant tidak mengizinkan terminal interaktif."}),
            ),
        ));
    }
    let max_sessions = plan.map(|p| p.max_terminal_sessions as usize).unwrap_or(2);
    Ok((
        context.user.id,
        context.linux_username,
        context.uid,
        context.uid,
        max_sessions,
    ))
}

/// Create and spawn a new PTY terminal session.
#[utoipa::path(
    post,
    path = "/api/v1/terminal/sessions",
    request_body = CreateTerminalSessionRequest,
    responses(
        (status = 201, description = "Terminal session initiated", body = TerminalSessionResponse),
        (status = 400, description = "Invalid dimensions or working directory"),
        (status = 403, description = "Forbidden: Access denied to target tenant"),
        (status = 429, description = "Tenant plan maximum terminal session quota exceeded")
    ),
    tag = "terminal"
)]
pub async fn create_session_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    headers: HeaderMap,
    Json(payload): Json<CreateTerminalSessionRequest>,
) -> Result<(StatusCode, Json<TerminalSessionResponse>), (StatusCode, Json<serde_json::Value>)> {
    // 1. Resolve target tenant username, uid, and gid
    let (target_user_id, linux_username, uid, gid, max_sessions) =
        resolve_terminal_user(&state, &user, payload.tenant.as_deref()).await?;

    // 2. Validate tenant plan concurrent session quota
    let active_count = state
        .terminal_hub
        .count_active_user_sessions(target_user_id)
        .await;

    if active_count >= max_sessions {
        warn!(
            "Tenant {} (user {}) exceeded terminal session quota ({} >= {})",
            linux_username, target_user_id, active_count, max_sessions
        );
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": format!(
                    "Batas kuota sesi terminal aktif tercapai ({}/{} sesi). Tutup sesi yang ada sebelum membuka sesi baru.",
                    active_count, max_sessions
                )
            })),
        ));
    }

    // 3. Normalize dimensions
    let cols = payload.cols.unwrap_or(DEFAULT_COLS).clamp(20, 500);
    let rows = payload.rows.unwrap_or(DEFAULT_ROWS).clamp(5, 200);

    // 4. Normalize and clean cwd
    let cwd = payload.cwd.as_deref().and_then(|c| {
        let trimmed = c.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    // 5. Spawn PTY session via hub & host agent
    let client_ip = crate::audit::extract_client_ip(&headers);

    let session = state
        .terminal_hub
        .create_session(
            target_user_id,
            user.username.clone(),
            linux_username.clone(),
            uid,
            gid,
            cwd.clone(),
            cols,
            rows,
            Some(max_sessions),
            Some(DEFAULT_IDLE_TIMEOUT_SECS),
            client_ip.clone(),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        })?;

    // 6. Record audit log entry for session start (P6-T4: catat awal sesi)
    let audit_details = serde_json::json!({
        "session_id": session.session_id,
        "tenant_user_id": target_user_id,
        "tenant": linux_username,
        "linux_username": linux_username,
        "folder": session.cwd,
        "cols": cols,
        "rows": rows,
        "pid": session.pid,
        "keystroke_recording": false,
    });

    state.audit_writer.log_async(
        Some(user.id),
        &user.username,
        "terminal.session_start",
        &session.session_id,
        "success",
        client_ip,
        Some(audit_details.to_string()),
    );

    info!(
        "Terminal session created: {} for user {} at {}",
        session.session_id, linux_username, session.cwd
    );

    Ok((StatusCode::CREATED, Json(session.to_response())))
}

/// List active terminal sessions belonging to the current user (or all if admin).
#[utoipa::path(
    get,
    path = "/api/v1/terminal/sessions",
    params(TerminalListQuery),
    responses(
        (status = 200, description = "List of active terminal sessions", body = TerminalSessionListResponse)
    ),
    tag = "terminal"
)]
pub async fn list_sessions_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Query(query): Query<TerminalListQuery>,
) -> Result<Json<TerminalSessionListResponse>, (StatusCode, Json<serde_json::Value>)> {
    let context =
        crate::tenant_context::resolve_tenant_context(&state, &user, query.tenant.as_deref(), None)
            .await?;
    let sessions = state
        .terminal_hub
        .list_sessions_for_user(context.user.id, false)
        .await
        .into_iter()
        .map(|s| s.to_response())
        .collect();

    Ok(Json(TerminalSessionListResponse { sessions }))
}

/// Get detailed status of a specific terminal session.
#[utoipa::path(
    get,
    path = "/api/v1/terminal/sessions/{id}",
    responses(
        (status = 200, description = "Terminal session metadata", body = TerminalSessionResponse),
        (status = 403, description = "Forbidden: Access denied"),
        (status = 404, description = "Session not found")
    ),
    tag = "terminal"
)]
pub async fn get_session_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(session_id): Path<String>,
) -> Result<Json<TerminalSessionResponse>, (StatusCode, Json<serde_json::Value>)> {
    let session = state
        .terminal_hub
        .get_session(&session_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Terminal session not found" })),
            )
        })?;

    if !user.is_admin() && session.user_id != user.id {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Access denied to terminal session" })),
        ));
    }

    Ok(Json(session.to_response()))
}

/// Terminate and close a terminal session.
#[utoipa::path(
    delete,
    path = "/api/v1/terminal/sessions/{id}",
    responses(
        (status = 200, description = "Session terminated", body = TerminalCloseApiResponse),
        (status = 403, description = "Forbidden: Access denied"),
        (status = 404, description = "Session not found")
    ),
    tag = "terminal"
)]
pub async fn close_session_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<TerminalCloseApiResponse>, (StatusCode, Json<serde_json::Value>)> {
    let session = state
        .terminal_hub
        .get_session(&session_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Terminal session not found" })),
            )
        })?;

    if !user.is_admin() && session.user_id != user.id {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Access denied to terminal session" })),
        ));
    }

    // Capture client IP from close request if not already populated
    let client_ip = crate::audit::extract_client_ip(&headers);
    if let Some(ip) = client_ip {
        let mut cur_ip = session.client_ip.write().await;
        if cur_ip.is_none() {
            *cur_ip = Some(ip);
        }
    }

    let closed = state
        .terminal_hub
        .close_session(&session_id, false)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        })?;

    Ok(Json(TerminalCloseApiResponse {
        session_id,
        closed,
        message: "Sesi terminal berhasil ditutup".to_string(),
    }))
}

/// Resize the terminal window for a session.
#[utoipa::path(
    post,
    path = "/api/v1/terminal/sessions/{id}/resize",
    request_body = TerminalResizeRequest,
    responses(
        (status = 200, description = "Window dimensions resized", body = TerminalResizeResponse),
        (status = 403, description = "Forbidden: Access denied"),
        (status = 404, description = "Session not found")
    ),
    tag = "terminal"
)]
pub async fn resize_session_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(session_id): Path<String>,
    Json(payload): Json<TerminalResizeRequest>,
) -> Result<Json<TerminalResizeResponse>, (StatusCode, Json<serde_json::Value>)> {
    let session = state
        .terminal_hub
        .get_session(&session_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Terminal session not found" })),
            )
        })?;

    if !user.is_admin() && session.user_id != user.id {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Access denied to terminal session" })),
        ));
    }

    let cols = payload.cols.clamp(20, 500);
    let rows = payload.rows.clamp(5, 200);

    state
        .terminal_hub
        .resize_session(&session_id, cols, rows)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        })?;

    Ok(Json(TerminalResizeResponse {
        session_id,
        cols,
        rows,
    }))
}

/// Query terminal audit logs (session start and end records) with folder and duration metrics.
#[utoipa::path(
    get,
    path = "/api/v1/terminal/audit",
    params(TerminalAuditQuery),
    responses(
        (status = 200, description = "List of terminal audit logs", body = TerminalAuditListResponse),
        (status = 403, description = "Forbidden: Insufficient permissions")
    ),
    tag = "terminal"
)]
pub async fn get_terminal_audit_handler(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Query(query): Query<TerminalAuditQuery>,
) -> Result<Json<TerminalAuditListResponse>, (StatusCode, Json<serde_json::Value>)> {
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * per_page;

    let (rows, total): (Vec<panel_db::models::AuditLog>, i64) = {
        let mut count_builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT COUNT(*) FROM audit_logs WHERE action LIKE 'terminal.session_%'",
        );
        let mut list_builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT id, user_id, actor_username, action, target_resource, details_json, ip_address, status, created_at FROM audit_logs WHERE action LIKE 'terminal.session_%'",
        );

        if !user.is_admin() {
            count_builder.push(" AND user_id = ");
            count_builder.push_bind(user.id);
            list_builder.push(" AND user_id = ");
            list_builder.push_bind(user.id);
        } else if let Some(tenant_filter) = &query.tenant {
            count_builder.push(" AND (actor_username = ");
            count_builder.push_bind(tenant_filter);
            count_builder.push(" OR details_json LIKE ");
            count_builder.push_bind(format!("%\"tenant\":\"{}\"%", tenant_filter));
            count_builder.push(")");

            list_builder.push(" AND (actor_username = ");
            list_builder.push_bind(tenant_filter);
            list_builder.push(" OR details_json LIKE ");
            list_builder.push_bind(format!("%\"tenant\":\"{}\"%", tenant_filter));
            list_builder.push(")");
        }

        list_builder.push(" ORDER BY created_at DESC, id DESC LIMIT ");
        list_builder.push_bind(per_page);
        list_builder.push(" OFFSET ");
        list_builder.push_bind(offset);

        let total: (i64,) = count_builder
            .build_query_as()
            .fetch_one(&state.operations_read_pool)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
            })?;

        let logs = list_builder
            .build_query_as::<panel_db::models::AuditLog>()
            .fetch_all(&state.operations_read_pool)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
            })?;

        (logs, total.0)
    };

    let items = rows
        .into_iter()
        .map(|log| {
            let parsed = log
                .details_json
                .as_deref()
                .and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok());
            let folder = parsed
                .as_ref()
                .and_then(|d| d.get("folder").and_then(|v| v.as_str()))
                .map(String::from);
            let duration_secs = parsed
                .as_ref()
                .and_then(|d| d.get("duration_secs").and_then(|v| v.as_i64()));
            let tenant = parsed
                .as_ref()
                .and_then(|d| d.get("tenant").and_then(|v| v.as_str()))
                .map(String::from)
                .or_else(|| {
                    parsed
                        .as_ref()
                        .and_then(|d| d.get("linux_username").and_then(|v| v.as_str()))
                        .map(String::from)
                });
            let keystroke_recording = parsed
                .as_ref()
                .and_then(|d| d.get("keystroke_recording").and_then(|v| v.as_bool()))
                .unwrap_or(false);

            TerminalAuditItem {
                id: log.id,
                session_id: log.target_resource,
                actor_username: log.actor_username,
                action: log.action,
                tenant,
                folder,
                duration_secs,
                ip_address: log.ip_address,
                status: log.status,
                keystroke_recording,
                created_at: log.created_at,
            }
        })
        .collect();

    let total_pages = if total == 0 {
        1
    } else {
        (total + per_page - 1) / per_page
    };

    Ok(Json(TerminalAuditListResponse {
        items,
        total,
        page,
        per_page,
        total_pages,
    }))
}
